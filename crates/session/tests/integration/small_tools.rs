//! The commands behind the smaller drawing and camera tools: `add_curve`,
//! `add_rotated_rectangle`, `look_around` and `position_camera`.

use glam::{DVec2, DVec3};
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

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn entities(d: &Document, kind: &str) -> Vec<Value> {
    let key = if kind == "vertex" {
        "vertices".into()
    } else {
        format!("{kind}s")
    };
    ask(d, json!({"type": "entities", "kinds": [kind]}))[key]
        .as_array()
        .unwrap()
        .clone()
}

fn vec3(v: &Value) -> DVec3 {
    DVec3::from(serde_json::from_value::<[f64; 3]>(v.clone()).unwrap())
}

fn view(d: &Document) -> (DVec3, DVec3, DVec3) {
    let c = ask(d, json!({"type": "camera"}));
    (vec3(&c["eye"]), vec3(&c["target"]), vec3(&c["up"]))
}

fn undo_depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

// add_curve

#[test]
fn add_curve_puts_an_open_polyline_on_one_curve() {
    let mut d = doc();
    let pts = json!([[0, 0, 0], [1, 0, 0], [1, 1, 0], [2, 1.5, 0]]);
    let out = run(&mut d, json!({"type": "add_curve", "points": pts})).unwrap();
    assert!(!out["changes"]["created"].as_array().unwrap().is_empty());
    let edges = entities(&d, "edge");
    assert_eq!(edges.len(), 3);
    let first = edges[0]["curve"].clone();
    assert!(first.is_string() && edges.iter().all(|e| e["curve"] == first));
    assert_eq!(undo_depth(&d), 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(entities(&d, "edge").is_empty());
}

#[test]
fn add_curve_closed_forms_a_face_and_curve_false_has_no_curve() {
    let mut d = doc();
    let pts = json!([[0, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 0]]);
    run(
        &mut d,
        json!({"type": "add_curve", "points": pts, "closed": true}),
    )
    .unwrap();
    assert_eq!(entities(&d, "face").len(), 1);
    assert!(entities(&d, "edge").iter().all(|e| e["curve"].is_string()));

    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_curve", "points": pts, "closed": true, "curve": false}),
    )
    .unwrap();
    assert_eq!(entities(&d, "face").len(), 1);
    assert!(
        entities(&d, "edge")
            .iter()
            .all(|e| e.get("curve").is_none())
    );
}

#[test]
fn add_curve_straight_polyline_has_no_curve_and_duplicates_drop() {
    let mut d = doc();
    let pts = json!([[0, 0, 0], [0, 0, 0], [1, 0, 0], [2, 0, 0]]);
    run(&mut d, json!({"type": "add_curve", "points": pts})).unwrap();
    assert!(
        entities(&d, "edge")
            .iter()
            .all(|e| e.get("curve").is_none())
    );
}

#[test]
fn add_curve_refuses_bad_input_and_changes_nothing() {
    let mut d = doc();
    let bad = [
        json!([[0, 0, 0]]),
        json!([]),
        json!([[0, 0, 0], [0, 0, 0]]),
        json!([[0, 0, 0], [1e9, 0, 0]]),
    ];
    for pts in bad {
        let e = run(&mut d, json!({"type": "add_curve", "points": pts})).unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{pts}");
    }
    // A closed polyline needs three distinct points.
    let e = run(
        &mut d,
        json!({"type": "add_curve", "points": [[0, 0, 0], [1, 0, 0], [0, 0, 0]], "closed": true}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(entities(&d, "edge").is_empty());
    assert_eq!(undo_depth(&d), 0);
}

#[test]
fn add_curve_refuses_too_many_points() {
    let mut d = doc();
    let many: Vec<[f64; 3]> = (0..10_001)
        .map(|i| [i as f64 * 0.01, (i % 2) as f64 * 0.01, 0.0])
        .collect();
    let e = run(&mut d, json!({"type": "add_curve", "points": many})).unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
}

// add_rotated_rectangle

#[test]
fn rotated_rectangle_is_a_face_toward_the_width_point() {
    let mut d = doc();
    // First side along (1,1,0)/sqrt2 for 2*sqrt2; width point 1 m off to its left.
    run(
        &mut d,
        json!({"type": "add_rotated_rectangle",
               "corner": [0, 0, 0], "edge_end": [2, 2, 0], "width_point": [-0.5, 1.5, 0]}),
    )
    .unwrap();
    let faces = entities(&d, "face");
    assert_eq!(faces.len(), 1);
    assert_eq!(entities(&d, "edge").len(), 4);
    let verts = entities(&d, "vertex");
    let got: Vec<DVec3> = verts.iter().map(|v| vec3(&v["position"])).collect();
    // The width is the distance from (-0.5, 1.5) to the line y = x.
    let w = DVec3::new(-0.5, 1.5, 0.0).distance(DVec3::new(0.5, 0.5, 0.0));
    let n = DVec3::new(-1.0, 1.0, 0.0).normalize() * w;
    let want = [
        DVec3::ZERO,
        DVec3::new(2.0, 2.0, 0.0),
        DVec3::new(2.0, 2.0, 0.0) + n,
        n,
    ];
    assert_eq!(got.len(), 4);
    for w in want {
        assert!(got.iter().any(|g| g.abs_diff_eq(w, 1e-9)), "{w} in {got:?}");
    }
    assert_eq!(undo_depth(&d), 1);
}

#[test]
fn rotated_rectangle_plane_follows_the_width_point() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rotated_rectangle",
               "corner": [0, 0, 0], "edge_end": [3, 0, 0], "width_point": [1, 0, 2]}),
    )
    .unwrap();
    let verts: Vec<DVec3> = entities(&d, "vertex")
        .iter()
        .map(|v| vec3(&v["position"]))
        .collect();
    assert!(
        verts
            .iter()
            .any(|v| v.abs_diff_eq(DVec3::new(3.0, 0.0, 2.0), 1e-9))
    );
    assert!(verts.iter().all(|v| v.y.abs() < 1e-9));
}

#[test]
fn rotated_rectangle_refuses_degenerate_input() {
    let mut d = doc();
    let cases = [
        // zero side
        json!([[0, 0, 0], [0, 0, 0], [0, 1, 0]]),
        // width point on the side's line
        json!([[0, 0, 0], [2, 0, 0], [5, 0, 0]]),
        json!([[0, 0, 0], [2, 0, 0], [1, 0, 0]]),
        // beyond the coordinate limit
        json!([[0, 0, 0], [2, 0, 0], [0, 1e9, 0]]),
        // too thin
        json!([[0, 0, 0], [2, 0, 0], [1, 1e-9, 0]]),
    ];
    for c in cases {
        let e = run(
            &mut d,
            json!({"type": "add_rotated_rectangle",
                   "corner": c[0], "edge_end": c[1], "width_point": c[2]}),
        )
        .unwrap_err();
        assert_eq!(e.0, ErrorKind::InvalidInput, "{c}");
    }
    assert!(entities(&d, "edge").is_empty());
    assert_eq!(undo_depth(&d), 0);
}

// look_around

fn level_view(d: &mut Document) {
    run(
        d,
        json!({"type": "set_camera", "eye": [0, -10, 1.7], "target": [0, 0, 1.7]}),
    )
    .unwrap();
}

#[test]
fn look_around_turns_about_the_eye() {
    let mut d = doc();
    level_view(&mut d);
    let (eye0, t0, _) = view(&d);
    let out = run(&mut d, json!({"type": "look_around", "yaw": 90})).unwrap();
    assert_eq!(out["changes"]["created"], json!([]));
    let (eye, target, up) = view(&d);
    assert!(eye.abs_diff_eq(eye0, 1e-9));
    // Counter-clockwise from above: +Y turns to -X.
    let fwd = (target - eye).normalize();
    assert!(fwd.abs_diff_eq(DVec3::new(-1.0, 0.0, 0.0), 1e-9), "{fwd}");
    assert!(((target - eye).length() - (t0 - eye0).length()).abs() < 1e-9);
    assert!(up.abs_diff_eq(DVec3::Z, 1e-9));
    run(&mut d, json!({"type": "look_around", "pitch": 30})).unwrap();
    let (eye, target, _) = view(&d);
    let fwd = (target - eye).normalize();
    assert!((fwd.z - 30f64.to_radians().sin()).abs() < 1e-9, "{fwd}");
    assert_eq!(undo_depth(&d), 0);
    assert_eq!(d.revision(), 0);
}

#[test]
fn look_around_clamps_pitch_and_keeps_up_valid() {
    let mut d = doc();
    level_view(&mut d);
    for pitch in [500.0, -500.0, 89.5] {
        run(&mut d, json!({"type": "look_around", "pitch": pitch})).unwrap();
        let (eye, target, up) = view(&d);
        assert!(up.is_finite() && up.abs_diff_eq(DVec3::Z, 1e-9));
        let fwd = (target - eye).normalize();
        assert!(fwd.is_finite());
        assert!(fwd.z.abs() <= 89f64.to_radians().sin() + 1e-9, "{fwd}");
    }
    // From a vertical view a pitch toward the horizon still gives a valid camera.
    run(&mut d, json!({"type": "standard_view", "view": "top"})).unwrap();
    run(
        &mut d,
        json!({"type": "look_around", "yaw": 10, "pitch": 20}),
    )
    .unwrap();
    let (eye, target, up) = view(&d);
    assert!(eye.is_finite() && target.is_finite() && up.is_finite());
    assert!(((target - eye).normalize().z).abs() <= 89f64.to_radians().sin() + 1e-9);
}

#[test]
fn look_around_refuses_non_finite_values() {
    let mut d = doc();
    let before = view(&d);
    let cmd: Command = serde_json::from_value(json!({"type": "look_around", "yaw": 1})).unwrap();
    let Command::LookAround { pitch, .. } = cmd else {
        panic!("not look_around")
    };
    assert_eq!(pitch, 0.0);
    let bad = Command::LookAround {
        yaw: f64::NAN,
        pitch: 0.0,
    };
    assert_eq!(d.execute(bad).unwrap_err().kind, ErrorKind::InvalidInput);
    let bad = Command::LookAround {
        yaw: 0.0,
        pitch: f64::INFINITY,
    };
    assert_eq!(d.execute(bad).unwrap_err().kind, ErrorKind::InvalidInput);
    assert_eq!(view(&d), before);
}

// position_camera

#[test]
fn position_camera_stands_at_the_point_looking_toward_look_at() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "position_camera", "point": [2, 3, 1], "look_at": [2, 10, 5]}),
    )
    .unwrap();
    assert_eq!(out["changes"]["created"], json!([]));
    let (eye, target, up) = view(&d);
    assert!(
        eye.abs_diff_eq(DVec3::new(2.0, 3.0, 1.0 + 1.676), 1e-9),
        "{eye}"
    );
    let fwd = (target - eye).normalize();
    // Level: the look_at height is ignored.
    assert!(fwd.abs_diff_eq(DVec3::Y, 1e-9), "{fwd}");
    assert!(up.abs_diff_eq(DVec3::Z, 1e-9));
    assert_eq!(undo_depth(&d), 0);
}

#[test]
fn position_camera_without_look_at_keeps_the_heading_and_goes_perspective() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "set_camera", "eye": [10, 0, 5], "target": [0, 0, 0], "projection": "parallel"}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "position_camera", "point": [1, 1, 0], "eye_height": 2}),
    )
    .unwrap();
    let (eye, target, _) = view(&d);
    assert!(eye.abs_diff_eq(DVec3::new(1.0, 1.0, 2.0), 1e-9));
    let fwd = (target - eye).normalize();
    assert!(fwd.abs_diff_eq(DVec3::NEG_X, 1e-9), "{fwd}");
    assert_eq!(
        ask(&d, json!({"type": "camera"}))["projection"],
        "perspective"
    );
}

#[test]
fn position_camera_from_a_vertical_view_uses_the_screen_up_as_heading() {
    let mut d = doc();
    run(&mut d, json!({"type": "standard_view", "view": "top"})).unwrap();
    run(
        &mut d,
        json!({"type": "position_camera", "point": [0, 0, 0]}),
    )
    .unwrap();
    let (eye, target, up) = view(&d);
    let fwd = (target - eye).normalize();
    assert!(fwd.is_finite() && fwd.z.abs() < 1e-9, "{fwd}");
    assert!(up.abs_diff_eq(DVec3::Z, 1e-9));
}

#[test]
fn position_camera_refuses_bad_input() {
    let mut d = doc();
    let before = view(&d);
    for c in [
        json!({"type": "position_camera", "point": [0, 0, 0], "eye_height": 0}),
        json!({"type": "position_camera", "point": [0, 0, 0], "eye_height": 20000}),
        json!({"type": "position_camera", "point": [0, 0, 0], "eye_height": -1}),
        json!({"type": "position_camera", "point": [1e9, 0, 0]}),
        json!({"type": "position_camera", "point": [0, 0, 0], "look_at": [0, 0, 5]}),
        json!({"type": "position_camera", "point": [0, 0, 0], "look_at": [0, 0, 1e9]}),
    ] {
        assert_eq!(
            run(&mut d, c.clone()).unwrap_err().0,
            ErrorKind::InvalidInput,
            "{c}"
        );
    }
    assert_eq!(view(&d), before);
}

#[test]
fn the_new_commands_classify_correctly() {
    for (v, camera) in [
        (json!({"type": "look_around", "yaw": 1}), true),
        (json!({"type": "position_camera", "point": [0, 0, 0]}), true),
        (
            json!({"type": "add_curve", "points": [[0, 0, 0], [1, 0, 0]]}),
            false,
        ),
        (
            json!({"type": "add_rotated_rectangle", "corner": [0, 0, 0],
                   "edge_end": [1, 0, 0], "width_point": [0, 1, 0]}),
            false,
        ),
    ] {
        let c: Command = serde_json::from_value(v).unwrap();
        assert_eq!(c.is_camera(), camera);
        assert_eq!(c.changes_model(), !camera);
    }
}

/// A smooth open path of `n` points: a circle's arc, three quarters round.
fn smooth_path(n: usize) -> Vec<[f64; 3]> {
    (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU * 0.75;
            [t.cos() * 5.0, t.sin() * 5.0, 0.0]
        })
        .collect()
}

#[test]
fn add_curve_draws_two_thousand_points_quickly() {
    let mut d = doc();
    let start = std::time::Instant::now();
    run(
        &mut d,
        json!({"type": "add_curve", "points": smooth_path(2000)}),
    )
    .unwrap();
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    let edges = entities(&d, "edge");
    assert_eq!(edges.len(), 1999);
    assert!(edges.iter().all(|e| e["curve"] == edges[0]["curve"]));
}

#[test]
#[ignore = "release-mode timing"]
fn add_curve_draws_ten_thousand_points_in_a_second() {
    let mut d = doc();
    let start = std::time::Instant::now();
    run(
        &mut d,
        json!({"type": "add_curve", "points": smooth_path(10_000)}),
    )
    .unwrap();
    assert!(start.elapsed().as_secs_f64() < 1.0, "{:?}", start.elapsed());
    assert_eq!(entities(&d, "edge").len(), 9999);
}

/// Points on a circle of radius 5, each about half a turn from the previous one: every chord
/// crosses nearly all the others.
fn star_path(n: usize) -> Vec<[f64; 3]> {
    (0..n)
        .map(|i| {
            let a = i as f64 * std::f64::consts::PI * (1.0 + 1.0 / n as f64);
            [a.cos() * 5.0, a.sin() * 5.0, 0.0]
        })
        .collect()
}

#[test]
fn a_curve_that_crosses_itself_too_often_is_refused_quickly() {
    let mut d = doc();
    let start = std::time::Instant::now();
    let e = run(
        &mut d,
        json!({"type": "add_curve", "points": star_path(400)}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("crosses itself too often"), "{}", e.1);
    assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    assert!(entities(&d, "edge").is_empty());
}

/// `n` parallel runs joined at alternate ends; `across` turns them a quarter turn.
fn serpentine(n: usize, across: bool) -> Vec<[f64; 3]> {
    let mut points = Vec::new();
    for i in 0..n {
        let (lo, hi) = if across { (-1.0, 5.0) } else { (0.0, 10.0) };
        let (start, end) = if i % 2 == 0 { (lo, hi) } else { (hi, lo) };
        let at = if across {
            0.5 + i as f64 * 0.02
        } else {
            i as f64 * 0.01
        };
        for run in [start, end] {
            points.push(if across {
                [at, run, 0.0]
            } else {
                [run, at, 0.0]
            });
        }
    }
    points
}

#[test]
fn a_curve_that_crosses_the_drawn_edges_too_often_is_refused_quickly() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_curve", "points": serpentine(400, false)}),
    )
    .unwrap();
    let edges = entities(&d, "edge").len();
    let start = std::time::Instant::now();
    let e = run(
        &mut d,
        json!({"type": "add_curve", "points": serpentine(400, true)}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(e.1.contains("crosses itself too often"), "{}", e.1);
    assert!(start.elapsed().as_secs() < 1, "{:?}", start.elapsed());
    assert_eq!(entities(&d, "edge").len(), edges);
}

#[test]
fn a_curve_that_crosses_the_drawn_edges_a_few_times_is_drawn() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_curve", "points": serpentine(10, false)}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_curve", "points": serpentine(10, true)}),
    )
    .unwrap();
}

#[test]
#[ignore = "release-mode timing"]
fn two_serpentines_are_refused_or_drawn_in_under_a_second() {
    for n in [60, 400] {
        let mut d = doc();
        let start = std::time::Instant::now();
        run(
            &mut d,
            json!({"type": "add_curve", "points": serpentine(n, false)}),
        )
        .unwrap();
        let _ = run(
            &mut d,
            json!({"type": "add_curve", "points": serpentine(n, true)}),
        );
        assert!(
            start.elapsed().as_secs_f64() < 1.0,
            "{n}: {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn a_curve_that_crosses_itself_a_few_times_is_drawn() {
    let mut d = doc();
    // A figure eight: one crossing.
    run(
        &mut d,
        json!({"type": "add_curve", "points": [[0, 0, 0], [2, 2, 0], [2, 0, 0], [0, 2, 0]]}),
    )
    .unwrap();
    assert_eq!(entities(&d, "edge").len(), 5);
    // A modest star: a few hundred crossings.
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_curve", "points": star_path(30)}),
    )
    .unwrap();
    assert!(entities(&d, "edge").len() > 100);
}

#[test]
#[ignore = "release-mode timing"]
fn add_curve_with_a_star_of_four_hundred_points_is_quick_either_way() {
    let mut d = doc();
    let start = std::time::Instant::now();
    let _ = run(
        &mut d,
        json!({"type": "add_curve", "points": star_path(400)}),
    );
    assert!(start.elapsed().as_secs_f64() < 1.0, "{:?}", start.elapsed());
    // The widest star that is still drawn (about the crossing cap) is quick too.
    let mut d = doc();
    let start = std::time::Instant::now();
    let _ = run(
        &mut d,
        json!({"type": "add_curve", "points": star_path(88)}),
    );
    assert!(start.elapsed().as_secs_f64() < 1.0, "{:?}", start.elapsed());
}

#[test]
fn add_curve_closed_collinear_points_are_refused() {
    let mut d = doc();
    let e = run(
        &mut d,
        json!({"type": "add_curve", "points": [[0, 0, 0], [1, 0, 0], [2, 0, 0]], "closed": true}),
    )
    .unwrap_err();
    assert_eq!(e.0, ErrorKind::InvalidInput);
    assert!(entities(&d, "edge").is_empty());
}

#[test]
fn a_pentagram_inside_a_faces_opening_fills_every_region() {
    let mut d = doc();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [20, 20, 0]}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [2, 2, 0], "opposite": [18, 18, 0]}),
    )
    .unwrap();
    let inner = entities(&d, "face")
        .into_iter()
        .find(|f| (f["area"].as_f64().unwrap() - 256.0).abs() < 1e-6)
        .expect("the inner face");
    run(
        &mut d,
        json!({"type": "erase", "ids": [inner["id"].clone()]}),
    )
    .unwrap();
    assert_eq!(entities(&d, "face").len(), 1, "only the ring is left");
    let star: Vec<[f64; 3]> = [0usize, 2, 4, 1, 3]
        .iter()
        .map(|k| {
            let a = std::f64::consts::TAU * *k as f64 / 5.0 + std::f64::consts::FRAC_PI_2;
            [10.0 + 7.0 * a.cos(), 10.0 + 7.0 * a.sin(), 0.0]
        })
        .collect();
    run(
        &mut d,
        json!({"type": "add_curve", "points": star, "closed": true, "curve": false}),
    )
    .unwrap();
    // The batch draws every side before faces form (draw_segments' documented rule), so the five
    // tips and the middle are all faces: the ring plus six regions.
    assert_eq!(entities(&d, "face").len(), 7);
}
