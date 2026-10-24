//! The `add_helix` and `copy_along_path` commands.

use glam::{DAffine3, DVec2, DVec3};
use kernel::{EntityId, Model};
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn command(v: Value) -> Result<Command, (ErrorKind, String)> {
    serde_json::from_value(v).map_err(|e| (ErrorKind::InvalidInput, e.to_string()))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    d.execute(command(v)?)
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

fn created(out: &Value, kind: char) -> Vec<Value> {
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| id.as_str().unwrap().starts_with(kind))
        .cloned()
        .collect()
}

fn helix(extra: Value) -> Value {
    let mut v = json!({
        "type": "add_helix", "center": [0, 0, 0], "radius": 1.0, "pitch": 0.5, "turns": 2,
        "segments": 8
    });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

/// The vertices, in order, of the one curve the edges are on.
fn curve_points(d: &Document, out: &Value) -> Vec<DVec3> {
    let m = d.model();
    let edges = created(out, 'e');
    let kernel_edge = |id: &Value| {
        let q = ask(d, json!({"type": "entity", "id": id}));
        q["id"].clone()
    };
    let _ = kernel_edge;
    let first = m
        .edges()
        .map(|(e, _)| e)
        .find(|e| {
            edges
                .iter()
                .any(|id| id.as_str() == Some(&EntityId::Edge(*e).to_string()))
        })
        .unwrap();
    let curve = m.curve_of(first).expect("the helix is a curve");
    assert!(!curve.closed);
    curve
        .vertices
        .iter()
        .map(|v| m.position(*v).unwrap())
        .collect()
}

fn near(a: DVec3, b: DVec3) -> bool {
    a.distance(b) < 1e-9
}

#[test]
fn a_helix_has_the_pitch_count_and_handedness_asked() {
    let mut d = doc();
    let out = run(&mut d, helix(json!({}))).unwrap();
    assert_eq!(depth(&d), 1);
    assert_eq!(created(&out, 'e').len(), 16);
    let mut pts = curve_points(&d, &out);
    // The curve runs in either direction; start at the end that is at the start point.
    if !near(pts[0], DVec3::X) {
        pts.reverse();
    }
    assert_eq!(pts.len(), 17);
    assert!(near(pts[0], DVec3::X));
    // A quarter of a turn on, counter-clockwise looking down the axis, a pitch/4 higher.
    assert!(near(pts[2], DVec3::new(0.0, 1.0, 0.125)), "{:?}", pts[2]);
    assert!(near(pts[8], DVec3::new(1.0, 0.0, 0.5)));
    assert!(near(pts[16], DVec3::new(1.0, 0.0, 1.0)));
    for p in &pts {
        assert!((DVec2::new(p.x, p.y).length() - 1.0).abs() < 1e-9);
    }
    // Not a polygon: its edges are on a curve whose edges are not polygon sides.
    assert!(d.model().edges().all(|(_, e)| !e.polygon));
}

#[test]
fn a_left_handed_helix_turns_the_other_way_and_the_start_follows_the_options() {
    let mut d = doc();
    let out = run(
        &mut d,
        helix(json!({"left_handed": true, "center": [1, 2, 3], "start_direction": [0, 1, 0]})),
    )
    .unwrap();
    let mut pts = curve_points(&d, &out);
    if !near(pts[0], DVec3::new(1.0, 3.0, 3.0)) {
        pts.reverse();
    }
    assert!(near(pts[0], DVec3::new(1.0, 3.0, 3.0)));
    // Clockwise looking down +z: from +y toward +x.
    assert!(near(pts[2], DVec3::new(2.0, 2.0, 3.125)), "{:?}", pts[2]);
}

#[test]
fn a_helix_about_another_axis_rises_along_it() {
    let mut d = doc();
    let out = run(&mut d, helix(json!({"axis": [1, 0, 0], "turns": 1.5}))).unwrap();
    let mut pts = curve_points(&d, &out);
    assert_eq!(pts.len(), 13);
    if pts[0].x.abs() > 1e-9 {
        pts.reverse();
    }
    assert!((pts[12].x - 0.75).abs() < 1e-9);
    assert!(
        pts.iter()
            .all(|p| (DVec2::new(p.y, p.z).length() - 1.0).abs() < 1e-9)
    );
}

#[test]
fn a_fractional_turn_count_ends_exactly_on_the_last_angle() {
    let mut d = doc();
    // 0.3 turns of 8 segments: 3 steps, 0.1 turn each.
    let out = run(&mut d, helix(json!({"turns": 0.3}))).unwrap();
    assert_eq!(created(&out, 'e').len(), 3);
    let mut pts = curve_points(&d, &out);
    if !near(pts[0], DVec3::X) {
        pts.reverse();
    }
    let a = 0.3 * std::f64::consts::TAU;
    assert!(
        near(pts[3], DVec3::new(a.cos(), a.sin(), 0.15)),
        "{:?}",
        pts[3]
    );
}

#[test]
fn bad_helix_options_are_refused_and_change_nothing() {
    let bad = [
        json!({"radius": 0}),
        json!({"radius": -1}),
        json!({"pitch": 0}),
        json!({"pitch": -0.5}),
        json!({"turns": 0}),
        json!({"turns": 1001}),
        json!({"segments": 2}),
        json!({"segments": 361}),
        json!({"axis": [0, 0, 0]}),
        json!({"start_direction": [0, 0, 0]}),
        json!({"start_direction": [0, 0, 1]}),
        json!({"turns": 1000, "segments": 24}),
        json!({"turns": 28, "segments": 360}),
        json!({"radius": 1e12}),
        json!({"center": [1e12, 0, 0]}),
    ];
    for extra in bad {
        let mut d = doc();
        let (kind, message) = run(&mut d, helix(extra.clone())).unwrap_err();
        assert_eq!(kind, ErrorKind::InvalidInput, "{extra}: {message}");
        assert_eq!(depth(&d), 0, "{extra}");
        assert_eq!(d.model().edges().count(), 0, "{extra}");
    }
    // The point limit itself is allowed: 9,999 steps make 10,000 points.
    let mut d = doc();
    run(
        &mut d,
        helix(json!({"turns": 27.775, "segments": 360, "radius": 1.0})),
    )
    .unwrap();
}

#[test]
fn the_helix_can_be_undone() {
    let mut d = doc();
    run(&mut d, helix(json!({}))).unwrap();
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(d.model().edges().count(), 0);
}

/// Every directed side is used once each way.
fn sides_pair_up(m: &Model) -> bool {
    let mut used = std::collections::BTreeMap::new();
    for (_, f) in m.faces() {
        for l in std::iter::once(&f.outer).chain(&f.holes) {
            for i in 0..l.len() {
                *used.entry((l[i], l[(i + 1) % l.len()])).or_insert(0) += 1;
            }
        }
    }
    !used.is_empty()
        && used
            .iter()
            .all(|((a, b), n)| *n == 1 && used.get(&(*b, *a)) == Some(&1))
}

#[test]
fn follow_me_along_a_helix_makes_a_closed_spring() {
    let mut d = doc();
    let out = run(
        &mut d,
        helix(json!({"radius": 1.0, "pitch": 0.4, "turns": 3, "segments": 24})),
    )
    .unwrap();
    let path = created(&out, 'e');
    assert_eq!(path.len(), 72);
    // A small circle at the start, square to the first edge.
    let step = std::f64::consts::TAU / 24.0;
    let first = DVec3::new(step.cos() - 1.0, step.sin(), 0.4 / 24.0).normalize();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [1, 0, 0], "radius": 0.05,
               "normal": first.to_array(), "segments": 12}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    assert!(sides_pair_up(m), "the spring is a closed solid");
    // 72 segments of 12 sides, and two caps.
    assert_eq!(
        m.faces().filter(|(_, f)| f.outer.len() == 4).count(),
        72 * 12
    );
    assert_eq!(m.faces().filter(|(_, f)| f.outer.len() == 12).count(), 2);
}

// ---- copy_along_path ----

/// An L-shaped open path (0,0,0) -> (4,0,0) -> (4,4,0); its edge ids.
fn l_path(d: &mut Document) -> Vec<Value> {
    run(
        d,
        json!({"type": "add_curve", "points": [[0,0,0],[4,0,0],[4,4,0]], "curve": false}),
    )
    .map(|o| created(&o, 'e'))
    .unwrap()
}

/// A small group at `at`; its instance id.
fn marker(d: &mut Document, at: [f64; 3]) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": at,
               "opposite": [at[0] + 0.1, at[1] + 0.1, at[2]]}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    let out = run(d, json!({"type": "make_group", "ids": [face]})).unwrap();
    out["placed"][0].clone()
}

fn transform_of(d: &Document, id: &Value) -> DAffine3 {
    let id: EntityId = serde_json::from_value(id.clone()).unwrap();
    let EntityId::Instance(i) = id else { panic!() };
    d.model().instance(i).unwrap().transform
}

fn placed(out: &Value) -> Vec<Value> {
    out["placed"].as_array().cloned().unwrap_or_default()
}

fn copy_along(ids: &[Value], path: &[Value], extra: Value) -> Value {
    let mut v = json!({"type": "copy_along_path", "ids": ids, "path": path});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

#[test]
fn copies_run_along_an_open_path_by_count_and_turn_at_the_corner() {
    let mut d = doc();
    let path = l_path(&mut d);
    let m = marker(&mut d, [-1.0, -1.0, 0.0]);
    let before = depth(&d);
    let out = run(&mut d, copy_along(&[m], &path, json!({"count": 5}))).unwrap();
    assert_eq!(depth(&d), before + 1);
    let placed = placed(&out);
    assert_eq!(placed.len(), 4);
    assert!(placed.windows(2).all(|w| w[0].as_str() < w[1].as_str()
        || w[0].as_str().unwrap().len() < w[1].as_str().unwrap().len()));
    // Length 8: positions 2, 4, 6, 8.
    let want = [
        (DVec3::new(2.0, 0.0, 0.0), DVec3::X),
        (DVec3::new(4.0, 0.0, 0.0), DVec3::Y),
        (DVec3::new(4.0, 2.0, 0.0), DVec3::Y),
        (DVec3::new(4.0, 4.0, 0.0), DVec3::Y),
    ];
    for (id, (at, along)) in placed.iter().zip(want) {
        let t = transform_of(&d, id);
        assert!(near(t.translation, at), "{:?} vs {at:?}", t.translation);
        assert!(near(t.matrix3.x_axis, along), "{:?}", t.matrix3.x_axis);
        assert!(near(t.matrix3.z_axis, DVec3::Z));
    }
}

#[test]
fn without_align_the_copies_only_move() {
    let mut d = doc();
    let path = l_path(&mut d);
    let m = marker(&mut d, [-1.0, -1.0, 0.0]);
    let out = run(
        &mut d,
        copy_along(&[m], &path, json!({"count": 3, "align": false})),
    )
    .unwrap();
    let placed = placed(&out);
    assert_eq!(placed.len(), 2);
    for (id, at) in placed
        .iter()
        .zip([DVec3::new(4.0, 0.0, 0.0), DVec3::new(4.0, 4.0, 0.0)])
    {
        let t = transform_of(&d, id);
        assert!(near(t.translation, at));
        assert!(near(t.matrix3.x_axis, DVec3::X));
    }
}

#[test]
fn spacing_includes_the_end_of_an_open_path() {
    for (spacing, copies) in [(2.0, 4), (3.0, 2), (8.0, 1), (9.0, 0), (1.0, 8)] {
        let mut d = doc();
        let path = l_path(&mut d);
        let m = marker(&mut d, [-1.0, -1.0, 0.0]);
        let before = depth(&d);
        let out = run(&mut d, copy_along(&[m], &path, json!({"spacing": spacing}))).unwrap();
        assert_eq!(placed(&out).len(), copies, "spacing {spacing}");
        assert_eq!(
            depth(&d),
            before + usize::from(copies > 0) as u64,
            "spacing {spacing}"
        );
    }
}

#[test]
fn one_copy_on_an_open_path_is_the_start_and_makes_nothing() {
    let mut d = doc();
    let path = l_path(&mut d);
    let m = marker(&mut d, [-1.0, -1.0, 0.0]);
    let before = depth(&d);
    let out = run(&mut d, copy_along(&[m], &path, json!({"count": 1}))).unwrap();
    assert!(placed(&out).is_empty());
    assert_eq!(depth(&d), before);
}

#[test]
fn the_start_is_the_end_of_an_open_path_nearest_the_originals() {
    let mut d = doc();
    let path = l_path(&mut d);
    let m = marker(&mut d, [4.0, 4.0, 0.0]);
    let out = run(
        &mut d,
        copy_along(&[m], &path, json!({"count": 3, "align": false})),
    )
    .unwrap();
    let placed = placed(&out);
    assert_eq!(placed.len(), 2);
    // From (4,4,0): to (4,0,0) and (0,0,0).
    for (id, by) in placed
        .iter()
        .zip([DVec3::new(0.0, -4.0, 0.0), DVec3::new(-4.0, -4.0, 0.0)])
    {
        assert!(near(transform_of(&d, id).translation, by));
    }
}

fn square_path(d: &mut Document) -> Vec<Value> {
    let out = run(
        d,
        json!({"type": "add_curve", "points": [[0,0,0],[4,0,0],[4,4,0],[0,4,0]],
               "closed": true, "curve": false}),
    )
    .unwrap();
    created(&out, 'e')
}

fn sorted(mut v: Vec<DVec3>) -> Vec<DVec3> {
    v.sort_by(|a, b| a.to_array().partial_cmp(&b.to_array()).unwrap());
    v
}

#[test]
fn copies_go_evenly_around_a_closed_path() {
    for (extra, n) in [(json!({"count": 4}), 3), (json!({"spacing": 4.0}), 3)] {
        let mut d = doc();
        let path = square_path(&mut d);
        let m = marker(&mut d, [-1.0, -1.0, 0.0]);
        let out = run(
            &mut d,
            copy_along(&[m], &path, {
                let mut e = extra.clone();
                e["align"] = json!(false);
                e
            }),
        )
        .unwrap();
        let at: Vec<DVec3> = placed(&out)
            .iter()
            .map(|i| transform_of(&d, i).translation)
            .collect();
        assert_eq!(at.len(), n, "{extra}");
        assert_eq!(
            sorted(at),
            sorted(vec![
                DVec3::new(4.0, 0.0, 0.0),
                DVec3::new(4.0, 4.0, 0.0),
                DVec3::new(0.0, 4.0, 0.0)
            ]),
            "{extra}"
        );
    }
}

#[test]
fn aligned_copies_on_a_closed_path_turn_with_it() {
    let mut d = doc();
    let path = square_path(&mut d);
    let m = marker(&mut d, [-1.0, -1.0, 0.0]);
    let out = run(&mut d, copy_along(&[m], &path, json!({"count": 8}))).unwrap();
    let placed = placed(&out);
    assert_eq!(placed.len(), 7);
    // Every copy's x axis is the direction the path leaves its position (a unit axis).
    let turns: Vec<DVec3> = placed
        .iter()
        .map(|i| transform_of(&d, i).matrix3.x_axis)
        .collect();
    let first = turns[0];
    assert!(near(first.abs(), DVec3::X) || near(first.abs(), DVec3::Y));
    assert!(
        turns
            .iter()
            .all(|t| near(t.abs(), DVec3::X) || near(t.abs(), DVec3::Y))
    );
    // Four distinct headings around a square.
    let mut distinct: Vec<DVec3> = Vec::new();
    for t in &turns {
        if !distinct.iter().any(|d| near(*d, *t)) {
            distinct.push(*t);
        }
    }
    assert_eq!(distinct.len(), 4);
}

#[test]
fn edges_and_faces_are_copied_along_a_path_too() {
    let mut d = doc();
    let path = l_path(&mut d);
    let out = run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0.5, 0], "opposite": [0.5, 1, 0]}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    run(
        &mut d,
        copy_along(&[face], &path, json!({"count": 3, "align": false})),
    )
    .unwrap();
    d.scene().validate().unwrap();
    // The original and two copies (one at x = 4, one at y = 4: the second touches the path
    // end and so may split path edges, but faces are three).
    assert_eq!(d.model().faces().count(), 3);
}

#[test]
fn bad_copy_along_path_requests_are_refused() {
    let mut d = doc();
    let path = l_path(&mut d);
    let m = marker(&mut d, [-1.0, -1.0, 0.0]);
    let before = depth(&d);
    let branch = {
        let out = run(
            &mut d,
            json!({"type": "add_edge", "start": [4, 0, 0], "end": [4, -3, 0]}),
        )
        .unwrap();
        created(&out, 'e')
    };
    let cases: Vec<(Value, ErrorKind)> = vec![
        (
            copy_along(std::slice::from_ref(&m), &path, json!({})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(
                std::slice::from_ref(&m),
                &path,
                json!({"count": 2, "spacing": 1.0}),
            ),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(std::slice::from_ref(&m), &path, json!({"count": 0})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(std::slice::from_ref(&m), &path, json!({"count": 10002})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(std::slice::from_ref(&m), &path, json!({"spacing": 0})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(std::slice::from_ref(&m), &path, json!({"spacing": -1})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(std::slice::from_ref(&m), &path, json!({"spacing": 1e-5})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(&[], &path, json!({"count": 2})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(std::slice::from_ref(&m), &[], json!({"count": 2})),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(
                std::slice::from_ref(&m),
                std::slice::from_ref(&m),
                json!({"count": 2}),
            ),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(
                std::slice::from_ref(&m),
                &[path[0].clone(), branch[0].clone(), path[1].clone()],
                json!({"count": 2}),
            ),
            ErrorKind::InvalidInput,
        ),
        (
            copy_along(&[json!("i999")], &path, json!({"count": 2})),
            ErrorKind::NotFound,
        ),
        (
            copy_along(
                std::slice::from_ref(&m),
                &[json!("e9999")],
                json!({"count": 2}),
            ),
            ErrorKind::NotFound,
        ),
    ];
    for (c, want) in cases {
        let (kind, message) = run(&mut d, c.clone()).unwrap_err();
        assert_eq!(kind, want, "{c}: {message}");
    }
    assert_eq!(depth(&d), before + 1, "only the branch edge was drawn");
}

#[test]
fn copies_beyond_the_coordinate_limit_are_refused() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [950000, 0, 0]}),
    )
    .unwrap();
    let path = created(&out, 'e');
    let m = marker(&mut d, [200000.0, -1.0, 0.0]);
    let before = depth(&d);
    let (kind, _) = run(
        &mut d,
        copy_along(&[m], &path, json!({"count": 2, "align": false})),
    )
    .unwrap_err();
    assert_eq!(kind, ErrorKind::InvalidInput);
    assert_eq!(depth(&d), before);
}

#[test]
fn copy_along_path_is_one_undo_step() {
    let mut d = doc();
    let path = l_path(&mut d);
    let m = marker(&mut d, [-1.0, -1.0, 0.0]);
    let before = depth(&d);
    run(&mut d, copy_along(&[m], &path, json!({"count": 4}))).unwrap();
    assert_eq!(depth(&d), before + 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    let all = ask(&d, json!({"type": "entities", "kinds": ["instance"]}));
    assert_eq!(all["instances"].as_array().unwrap().len(), 1);
}

#[test]
fn the_new_commands_take_batch_references() {
    let mut d = doc();
    let batch: Vec<session::BatchCommand> = [
        helix(json!({})),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [0.1, 0.1, 0]}),
        json!({"type": "make_group", "ids": ["$1:*"]}),
        json!({"type": "copy_along_path", "ids": ["$2:i0"], "path": ["$0:e*"], "count": 3}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| session::BatchCommand::parse(v, i).unwrap())
    .collect();
    let out = d.execute_batch(batch, true).unwrap();
    assert_eq!(out[3].placed.as_ref().unwrap().len(), 2);
}
