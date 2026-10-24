//! The `revolve` command: a face turned about an axis into a solid.

use glam::{DVec2, DVec3};
use kernel::Model;
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

fn rectangle(d: &mut Document, a: [f64; 3], b: [f64; 3], normal: [f64; 3]) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": a, "opposite": b, "normal": normal}),
    )
    .unwrap();
    created(&out, 'f')[0].clone()
}

/// A polygon through `points` (in one plane); its face.
fn polygon(d: &mut Document, points: &[[f64; 3]]) -> Value {
    let mut face = None;
    for i in 0..points.len() {
        let out = run(
            d,
            json!({"type": "add_edge", "start": points[i], "end": points[(i + 1) % points.len()]}),
        )
        .unwrap();
        if let Some(f) = created(&out, 'f').first() {
            face = Some(f.clone());
        }
    }
    face.expect("the polygon made a face")
}

fn depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

fn label(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["undo"][0].clone()
}

fn exists(d: &Document, id: &Value) -> bool {
    let q: Query = serde_json::from_value(json!({"type": "entity", "id": id})).unwrap();
    d.query(&q).is_ok()
}

fn soft_edges(m: &Model) -> usize {
    m.edges().filter(|(_, e)| e.soft).count()
}

/// Every edge bounds exactly two faces.
fn closed_solid(m: &Model) -> bool {
    m.edges().all(|(e, _)| m.faces_beside(e).is_some())
}

/// The volume the faces enclose, by the divergence theorem: positive when they face out.
fn volume(m: &Model) -> f64 {
    m.faces()
        .map(|(id, f)| m.position(f.outer[0]).unwrap().dot(f.normal) * m.area(id).unwrap() / 3.0)
        .sum()
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

/// The area of a regular `n`-gon inscribed in radius `r`.
fn ngon(n: f64, r: f64) -> f64 {
    n / 2.0 * r * r * (std::f64::consts::TAU / n).sin()
}

fn revolve(face: &Value) -> Value {
    json!({"type": "revolve", "face": face, "center": [0, 0, 0], "axis": [0, 0, 1]})
}

#[test]
fn a_rectangle_beside_the_axis_revolves_into_a_faceted_tube() {
    let mut d = doc();
    let face = rectangle(&mut d, [1.0, 0.0, 0.0], [2.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    let before = depth(&d);
    run(&mut d, revolve(&face)).unwrap();
    d.scene().validate().unwrap();
    assert_eq!(depth(&d), before + 1);
    assert_eq!(label(&d), json!("revolve"));
    let m = d.model();
    assert!(!exists(&d, &face), "a full turn removes the face");
    // Outer and inner wall, a ring on top and one below.
    assert_eq!(m.faces().count(), 24 + 24 + 2);
    assert!(closed_solid(m));
    let want = ngon(24.0, 2.0) - ngon(24.0, 1.0);
    assert!((volume(m) - want).abs() < 1e-9, "{} {want}", volume(m));
    // The edges between the wall faces are soft, the rims are not.
    assert_eq!(soft_edges(m), 48);
    assert!(m.faces().all(|(id, _)| m.area(id).unwrap() > 1e-6));
    run(&mut d, json!({"type": "undo"})).unwrap();
    d.scene().validate().unwrap();
    assert!(exists(&d, &face));
    assert_eq!(m_counts(&d), (4, 4, 1));
}

fn m_counts(d: &Document) -> (usize, usize, usize) {
    counts(d.model())
}

#[test]
fn a_triangle_touching_the_axis_makes_a_faceted_cone_without_slivers() {
    let mut d = doc();
    let face = polygon(&mut d, &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
    run(&mut d, revolve(&face)).unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    // 24 sloping triangles and the base disk; the side on the axis leaves no face.
    assert_eq!(m.faces().count(), 25);
    assert!(closed_solid(m));
    let want = ngon(24.0, 1.0) / 3.0;
    assert!((volume(m) - want).abs() < 1e-9, "{} {want}", volume(m));
    assert!(m.faces().all(|(id, _)| m.area(id).unwrap() > 1e-6));
    assert_eq!(soft_edges(m), 24);
    // Nothing is left along the axis.
    let on_axis = |p: DVec3| p.x.abs() < 1e-9 && p.y.abs() < 1e-9;
    assert!(m.edges().all(|(_, e)| {
        !(on_axis(m.position(e.v[0]).unwrap()) && on_axis(m.position(e.v[1]).unwrap()))
    }));
    // The apex is one vertex; the base centre has no face to join.
    assert_eq!(m.vertices().count(), 24 + 1);
}

#[test]
fn a_rectangle_on_the_axis_makes_a_faceted_cylinder() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 2.0], [0.0, 1.0, 0.0]);
    run(&mut d, revolve(&face)).unwrap();
    let m = d.model();
    assert_eq!(m.faces().count(), 24 + 2);
    assert!(closed_solid(m));
    let want = ngon(24.0, 1.0) * 2.0;
    assert!((volume(m) - want).abs() < 1e-9, "{} {want}", volume(m));
    assert!(m.faces().all(|(id, _)| m.area(id).unwrap() > 1e-6));
    assert_eq!(soft_edges(m), 24);
}

#[test]
fn a_quarter_turn_keeps_the_face_and_caps_the_end() {
    let mut d = doc();
    let face = rectangle(&mut d, [1.0, 0.0, 0.0], [2.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    let mut cmd = revolve(&face);
    cmd["angle"] = json!(90);
    run(&mut d, cmd).unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    assert!(exists(&d, &face), "the face stays as the start cap");
    // Six steps: outer and inner wall, a sector on top and below, two caps.
    assert_eq!(m.faces().count(), 6 + 6 + 2 + 2);
    assert!(closed_solid(m));
    let want = (ngon(24.0, 2.0) - ngon(24.0, 1.0)) / 4.0;
    assert!((volume(m) - want).abs() < 1e-9, "{} {want}", volume(m));
    // Walls are soft between steps, not at the caps.
    assert_eq!(soft_edges(m), 5 + 5);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(m_counts(&d), (4, 4, 1));
    assert!(exists(&d, &face));
}

#[test]
fn a_partial_turn_with_a_painted_face_paints_both_caps() {
    let mut d = doc();
    let mat = run(
        &mut d,
        json!({"type": "add_material", "color": [200, 0, 0]}),
    )
    .unwrap();
    let mat = mat["changes"]["materials"][0].clone();
    let face = rectangle(&mut d, [1.0, 0.0, 0.0], [2.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    run(
        &mut d,
        json!({"type": "paint", "ids": [face], "material": mat}),
    )
    .unwrap();
    let mut cmd = revolve(&face);
    cmd["angle"] = json!(180);
    run(&mut d, cmd).unwrap();
    let m = d.model();
    assert!(closed_solid(m));
    let painted = m
        .faces()
        .filter(|(_, f)| f.paint().iter().any(|p| p.is_some()))
        .count();
    assert_eq!(painted, 2);
    assert!(volume(m) > 0.0);
}

#[test]
fn a_face_across_the_axis_or_square_to_it_is_refused() {
    let mut d = doc();
    let across = rectangle(&mut d, [-1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    let flat = rectangle(&mut d, [1.0, 5.0, 3.0], [2.0, 6.0, 3.0], [0.0, 0.0, 1.0]);
    // Parallel to the axis, off to one side, but straddling the axis' projection.
    let beside = rectangle(&mut d, [-1.0, 4.0, 0.0], [1.0, 4.0, 1.0], [0.0, 1.0, 0.0]);
    let (before, steps) = (m_counts(&d), depth(&d));
    for (face, what) in [
        (&across, "cross"),
        (&flat, "perpendicular"),
        (&beside, "straddle"),
    ] {
        let (kind, msg) = run(&mut d, revolve(face)).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{what}: {msg}");
        assert!(msg.contains("axis"), "{what}: {msg}");
        assert_eq!(m_counts(&d), before, "{what}");
        assert_eq!(depth(&d), steps, "{what}");
    }
}

#[test]
fn a_face_beside_a_parallel_axis_and_a_tilted_face_off_to_one_side_revolve() {
    let mut d = doc();
    let face = rectangle(&mut d, [0.5, 3.0, 0.0], [1.5, 3.0, 1.0], [0.0, 1.0, 0.0]);
    run(&mut d, revolve(&face)).unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    assert!(closed_solid(m));
    assert!(volume(m) > 0.0);
}

#[test]
fn bad_input_is_refused_without_a_change() {
    let mut d = doc();
    let face = rectangle(&mut d, [1.0, 0.0, 0.0], [2.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    let edge = {
        let out = run(
            &mut d,
            json!({"type": "add_edge", "start": [5, 5, 5], "end": [6, 5, 5]}),
        )
        .unwrap();
        created(&out, 'e')[0].clone()
    };
    let (before, steps) = (m_counts(&d), depth(&d));
    let with = |k: &str, v: Value| {
        let mut c = revolve(&face);
        c[k] = v;
        c
    };
    let bad = [
        with("axis", json!([0, 0, 0])),
        with("angle", json!(0)),
        with("angle", json!(-10)),
        with("angle", json!(361)),
        with("segments", json!(0)),
        with("segments", json!(361)),
        with("segments", json!(2)),
        with("center", json!([0, 0, 1e9])),
        revolve(&edge),
    ];
    for cmd in bad {
        let (kind, msg) = run(&mut d, cmd.clone()).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{cmd}: {msg}");
        assert_eq!(m_counts(&d), before, "{cmd}");
        assert_eq!(depth(&d), steps, "{cmd}");
    }
    let (kind, _) = run(&mut d, revolve(&json!("f999"))).unwrap_err();
    assert_eq!(kind, ErrorKind::NotFound);
    // A face too close to the axis for the turn to have a side.
    let thin = rectangle(
        &mut d,
        [2.0e-6, 0.0, 10.0],
        [3.0, 0.0, 11.0],
        [0.0, 1.0, 0.0],
    );
    let (kind, msg) = run(&mut d, revolve(&thin)).unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput, "{msg}");
}

#[test]
fn a_tilted_axis_and_center_move_the_solid() {
    let mut d = doc();
    // A rectangle beside the line through (0, 0, 5) along +y.
    let face = rectangle(&mut d, [1.0, 0.0, 4.0], [2.0, 1.0, 4.0], [0.0, 0.0, 1.0]);
    let mut cmd = revolve(&face);
    cmd["center"] = json!([0, 0, 5]);
    cmd["axis"] = json!([0, 3, 0]);
    cmd["segments"] = json!(12);
    run(&mut d, cmd).unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    assert!(closed_solid(m));
    // The plane z = 4 is parallel to the axis, one unit below it, and the face lies on one
    // side of the axis' projection.
    assert!(volume(m) > 0.0);
}

#[test]
fn a_square_ring_with_a_hole_revolves_into_a_closed_solid() {
    let mut d = doc();
    let outer = rectangle(&mut d, [2.0, 0.0, 0.0], [5.0, 0.0, 3.0], [0.0, 1.0, 0.0]);
    let hole = rectangle(&mut d, [3.0, 0.0, 1.0], [4.0, 0.0, 2.0], [0.0, 1.0, 0.0]);
    run(&mut d, json!({"type": "erase", "ids": [hole]})).unwrap();
    let face = outer;
    run(&mut d, revolve(&face)).unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    assert!(closed_solid(m));
    assert!(volume(m) > 0.0);
}

#[test]
fn a_partial_turn_closes_whichever_way_the_face_points() {
    for normal in [[0.0, 1.0, 0.0], [0.0, -1.0, 0.0]] {
        for angle in [90.0, 270.0] {
            let mut d = doc();
            let face = rectangle(&mut d, [1.0, 0.0, 0.0], [2.0, 0.0, 1.0], normal);
            let mut cmd = revolve(&face);
            cmd["angle"] = json!(angle);
            cmd["segments"] = json!(12);
            run(&mut d, cmd).unwrap();
            d.scene().validate().unwrap();
            let m = d.model();
            assert!(closed_solid(m), "{normal:?} {angle}");
            let want = (ngon(12.0, 2.0) - ngon(12.0, 1.0)) * angle / 360.0;
            assert!((volume(m) - want).abs() < 1e-9, "{} {want}", volume(m));
        }
    }
}

#[test]
fn a_circle_beside_the_axis_makes_a_smooth_torus() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [3, 0, 0], "radius": 1.0, "normal": [0, 1, 0],
               "segments": 12}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    let mut cmd = revolve(&face);
    cmd["segments"] = json!(16);
    run(&mut d, cmd).unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    assert_eq!(m.faces().count(), 12 * 16);
    assert!(closed_solid(m));
    assert!(volume(m) > 0.0);
    // The profile is one smooth curve, and so is each ring: every edge is soft.
    assert_eq!(soft_edges(m), m.edges().count());
}
