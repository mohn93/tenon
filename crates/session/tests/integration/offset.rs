//! The `offset` command: faces and chains of edges offset in their plane.

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command =
        serde_json::from_value(v).map_err(|e| (ErrorKind::InvalidInput, e.to_string()))?;
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn created(out: &Value, kind: char) -> Vec<Value> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with(kind))
        .cloned()
        .collect()
}

fn faces(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .clone()
}

/// The faces' areas, smallest first.
fn areas(d: &Document) -> Vec<f64> {
    let mut a: Vec<f64> = faces(d)
        .iter()
        .map(|f| f["area"].as_f64().unwrap())
        .collect();
    a.sort_by(f64::total_cmp);
    a
}

fn close_all(got: &[f64], want: &[f64]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9)
}

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

/// The ends of `edges`, as [x, y, z].
fn ends(d: &Document, edges: &[Value]) -> Vec<[f64; 3]> {
    edges
        .iter()
        .flat_map(|e| {
            let info = ask(d, json!({"type": "entity", "id": e}));
            let start: [f64; 3] = serde_json::from_value(info["start"].clone()).unwrap();
            let end: [f64; 3] = serde_json::from_value(info["end"].clone()).unwrap();
            [start, end]
        })
        .collect()
}

fn rectangle(d: &mut Document, corner: [f64; 3], opposite: [f64; 3]) {
    run(
        d,
        json!({"type": "add_rectangle", "corner": corner, "opposite": opposite}),
    )
    .unwrap();
}

/// Draws the polyline through `points` and returns its edges in order.
fn polyline(d: &mut Document, points: &[[f64; 3]]) -> Vec<Value> {
    points
        .windows(2)
        .map(|w| {
            let out = run(d, json!({"type": "add_edge", "start": w[0], "end": w[1]})).unwrap();
            created(&out, 'e')[0].clone()
        })
        .collect()
}

#[test]
fn a_face_offset_inward_splits_into_the_inner_face_and_a_ring() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let face = faces(&d)[0]["id"].clone();
    let before = depth(&d);
    let out = run(
        &mut d,
        json!({"type": "offset", "ids": [face], "distance": 0.5}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    assert_eq!(edges.len(), 4, "{out}");
    for p in ends(&d, &edges) {
        assert!(
            (p[0] - 0.5).abs() < 1e-12 || (p[0] - 1.5).abs() < 1e-12,
            "{p:?}"
        );
        assert!(
            (p[1] - 0.5).abs() < 1e-12 || (p[1] - 1.5).abs() < 1e-12,
            "{p:?}"
        );
    }
    assert!(close_all(&areas(&d), &[1.0, 3.0]), "{:?}", areas(&d));
    assert_eq!(depth(&d), before + 1);
    let h = ask(&d, json!({"type": "history"}));
    assert_eq!(h["undo"][0], "offset");

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(close_all(&areas(&d), &[4.0]), "{:?}", areas(&d));
    assert_eq!(
        ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn a_face_offset_outward_draws_the_outer_loop_and_the_ring() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let face = faces(&d)[0]["id"].clone();
    let out = run(
        &mut d,
        json!({"type": "offset", "ids": [face], "distance": -0.5}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    assert_eq!(edges.len(), 4, "{out}");
    for p in ends(&d, &edges) {
        assert!(
            (p[0] + 0.5).abs() < 1e-12 || (p[0] - 2.5).abs() < 1e-12,
            "{p:?}"
        );
    }
    assert!(close_all(&areas(&d), &[4.0, 5.0]), "{:?}", areas(&d));
}

#[test]
fn a_face_with_a_hole_shrinks_its_outline_and_grows_its_hole() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [4.0, 4.0, 0.0]);
    rectangle(&mut d, [1.5, 1.5, 0.0], [2.5, 2.5, 0.0]);
    let holed = faces(&d)
        .into_iter()
        .find(|f| !f["holes"].as_array().unwrap().is_empty())
        .unwrap();
    let out = run(
        &mut d,
        json!({"type": "offset", "ids": [holed["id"]], "distance": 0.5}),
    )
    .unwrap();
    assert_eq!(created(&out, 'e').len(), 8, "{out}");
    // Outline ring 16 - 9, the ring between the new loops 9 - 4, hole ring 4 - 1, the inner
    // square 1.
    assert!(
        close_all(&areas(&d), &[1.0, 3.0, 5.0, 7.0]),
        "{:?}",
        areas(&d)
    );
    let ring = faces(&d)
        .into_iter()
        .find(|f| (f["area"].as_f64().unwrap() - 5.0).abs() < 1e-9)
        .unwrap();
    assert_eq!(ring["holes"].as_array().unwrap().len(), 1, "{ring}");
    // Grown too far, the hole meets the outline.
    let before = ask(&d, json!({"type": "entities"}));
    let (kind, msg) = run(
        &mut d,
        json!({"type": "offset", "ids": [ring["id"]], "distance": 0.5}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert!(msg.contains("too large"), "{msg}");
    assert_eq!(ask(&d, json!({"type": "entities"})), before);
}

#[test]
fn a_circle_offsets_into_a_circle_of_as_many_segments() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0}),
    )
    .unwrap();
    let face = faces(&d)[0]["id"].clone();
    let out = run(
        &mut d,
        json!({"type": "offset", "ids": [face], "distance": 0.2}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    assert_eq!(edges.len(), 24, "{out}");
    let c = ask(&d, json!({"type": "curve", "id": edges[0]}));
    assert_eq!(c["kind"], "circle", "{c}");
    assert_eq!(c["segments"], 24, "{c}");
    let r = 1.0 - 0.2 / (std::f64::consts::PI / 24.0).cos();
    assert!((c["radius"].as_f64().unwrap() - r).abs() < 1e-9, "{c}");
}

#[test]
fn a_polygon_offsets_into_a_polygon() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_polygon", "center": [0, 0, 0], "radius": 1.0, "sides": 5}),
    )
    .unwrap();
    let face = faces(&d)[0]["id"].clone();
    let out = run(
        &mut d,
        json!({"type": "offset", "ids": [face], "distance": -0.2}),
    )
    .unwrap();
    let edges = created(&out, 'e');
    assert_eq!(edges.len(), 5, "{out}");
    let c = ask(&d, json!({"type": "curve", "id": edges[0]}));
    assert_eq!(c["kind"], "polygon", "{c}");
    assert_eq!(c["segments"], 5, "{c}");
}

#[test]
fn an_open_chain_offsets_to_the_side_of_toward() {
    let u = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    // Inside the U, and outside it; near the top side, which walks back the other way.
    for (toward, inside) in [
        ([1.0, 0.5, 0.0], true),
        ([1.0, 0.95, 0.0], true),
        ([3.0, 0.5, 0.0], false),
        ([1.0, 1.5, 0.0], false),
    ] {
        let mut d = doc();
        let chain = polyline(&mut d, &u);
        let out = run(
            &mut d,
            json!({"type": "offset", "ids": chain, "distance": 0.1, "toward": toward}),
        )
        .unwrap();
        let edges = created(&out, 'e');
        assert_eq!(edges.len(), 3, "{out}");
        for p in ends(&d, &edges) {
            let within = p[0] <= 1.9 + 1e-12 && p[1] >= 0.1 - 1e-12 && p[1] <= 0.9 + 1e-12;
            let without = p[0] >= -1e-12
                && (p[0] >= 2.1 - 1e-12 || p[1] <= -0.1 + 1e-12 || p[1] >= 1.1 - 1e-12);
            assert!(if inside { within } else { without }, "{toward:?}: {p:?}");
        }
        assert!(faces(&d).is_empty());
    }
}

#[test]
fn a_closed_chain_of_edges_offsets_to_the_side_of_toward() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let edges: Vec<Value> = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    let out = run(
        &mut d,
        json!({"type": "offset", "ids": edges, "distance": 0.5, "toward": [3, 1, 0]}),
    )
    .unwrap();
    assert_eq!(created(&out, 'e').len(), 4, "{out}");
    assert!(close_all(&areas(&d), &[4.0, 5.0]), "{:?}", areas(&d));
}

#[test]
fn bad_offsets_are_refused_and_change_nothing() {
    let mut d = doc();
    rectangle(&mut d, [0.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    let face = faces(&d)[0]["id"].clone();
    let apart = polyline(&mut d, &[[5.0, 0.0, 0.0], [6.0, 0.0, 0.0]]);
    let other = polyline(&mut d, &[[8.0, 0.0, 0.0], [9.0, 0.0, 0.0]]);
    let tee = polyline(
        &mut d,
        &[[10.0, 0.0, 0.0], [11.0, 0.0, 0.0], [12.0, 0.0, 0.0]],
    );
    let stem = polyline(&mut d, &[[11.0, 0.0, 0.0], [11.0, 1.0, 0.0]]);
    let bent = polyline(
        &mut d,
        &[
            [20.0, 0.0, 0.0],
            [21.0, 0.0, 0.0],
            [21.0, 1.0, 0.0],
            [21.0, 1.0, 1.0],
        ],
    );
    let two = [apart[0].clone(), other[0].clone()];
    let branch = [tee[0].clone(), tee[1].clone(), stem[0].clone()];
    let before = ask(&d, json!({"type": "entities"}));
    let steps = depth(&d);
    for bad in [
        json!({"type": "offset", "ids": [face], "distance": 0}),
        json!({"type": "offset", "ids": [face], "distance": 1e-6}),
        json!({"type": "offset", "ids": [face], "distance": 1.0}),
        json!({"type": "offset", "ids": [face], "distance": 5.0}),
        json!({"type": "offset", "ids": [face], "distance": 0.1, "toward": [1, 1, 0]}),
        json!({"type": "offset", "ids": [], "distance": 0.1}),
        json!({"type": "offset", "ids": two, "distance": 0.1, "toward": [7, 1, 0]}),
        json!({"type": "offset", "ids": branch, "distance": 0.1, "toward": [10.5, 0.5, 0]}),
        json!({"type": "offset", "ids": apart, "distance": -0.1, "toward": [5.5, 1, 0]}),
        json!({"type": "offset", "ids": apart, "distance": 0.1}),
        json!({"type": "offset", "ids": apart, "distance": 0.1, "toward": [5.5, 0, 0]}),
        json!({"type": "offset", "ids": [apart[0], face], "distance": 0.1}),
        json!({"type": "offset", "ids": bent, "distance": 0.1, "toward": [20.5, 0.5, 0]}),
    ] {
        let (kind, msg) = run(&mut d, bad.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{bad}: {msg}");
        assert_eq!(ask(&d, json!({"type": "entities"})), before, "{bad}");
        assert_eq!(depth(&d), steps, "{bad}");
    }
    let (kind, _) = run(
        &mut d,
        json!({"type": "offset", "ids": ["f999"], "distance": 0.1}),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
}
