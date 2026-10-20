//! The `follow_me` command: a face swept along a chain of edges.

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

fn edge(d: &mut Document, a: [f64; 3], b: [f64; 3]) -> Value {
    let out = run(d, json!({"type": "add_edge", "start": a, "end": b})).unwrap();
    created(&out, 'e')[0].clone()
}

/// A rectangle with corners `a` and `b` square to `normal`; its face.
fn rectangle(d: &mut Document, a: [f64; 3], b: [f64; 3], normal: [f64; 3]) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": a, "opposite": b, "normal": normal}),
    )
    .unwrap();
    created(&out, 'f')[0].clone()
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

/// The volume the faces (all but `skip`) enclose, by the divergence theorem: positive when
/// they all face out.
fn volume(m: &Model, skip: &[kernel::FaceId]) -> f64 {
    m.faces()
        .filter(|(id, _)| !skip.contains(id))
        .map(|(id, f)| m.position(f.outer[0]).unwrap().dot(f.normal) * m.area(id).unwrap() / 3.0)
        .sum()
}

fn has_vertex(m: &Model, p: DVec3) -> bool {
    m.vertices().any(|(_, v)| v.pos.distance(p) < 1e-9)
}

/// The model's counts of vertices, edges and faces.
fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

#[test]
fn a_square_along_an_l_makes_a_closed_mitred_solid_in_one_step() {
    let mut d = doc();
    let path = [
        edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        edge(&mut d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]),
    ];
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
    let before = depth(&d);
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    assert_eq!(depth(&d), before + 1);
    assert_eq!(label(&d), json!("follow_me"));
    let m = d.model();
    assert!(!exists(&d, &face), "the profile face is gone");
    // Four sides on each of two segments, and two caps.
    assert_eq!(m.faces().count(), 10);
    assert!(closed_solid(m));
    assert!((volume(m, &[]) - 3.0).abs() < 1e-9, "{}", volume(m, &[]));
    // The corner is mitred: the inside of the turn meets at x + y = 2.
    assert!(has_vertex(m, DVec3::new(1.0, 1.0, 0.0)));
    assert!(has_vertex(m, DVec3::new(1.0, 1.0, 1.0)));
    assert_eq!(soft_edges(m), 0);
    run(&mut d, json!({"type": "undo"})).unwrap();
    d.scene().validate().unwrap();
    assert!(exists(&d, &face));
    assert_eq!(d.model().faces().count(), 1);
}

#[test]
fn a_rectangle_around_a_circle_makes_a_smooth_ring_without_caps() {
    let mut d = doc();
    let out = run(
        &mut d,
        json!({"type": "add_circle", "center": [0, 0, 0], "radius": 1.0, "segments": 24}),
    )
    .unwrap();
    let path = created(&out, 'e');
    let disk = created(&out, 'f')[0].clone();
    assert_eq!(path.len(), 24);
    let face = rectangle(&mut d, [1.5, 0.0, 0.5], [2.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    assert!(!exists(&d, &face), "the profile face is gone");
    let m = d.model();
    // Four sides on each of 24 segments, no caps, and the disk.
    assert_eq!(m.faces().count(), 97);
    // The joints between segments are soft (the path is a circle), the profile's corners hard.
    assert_eq!(soft_edges(m), 96);
    for (e, edge) in m.edges() {
        if edge.soft {
            assert!(m.faces_beside(e).is_some());
        }
    }
    let disk_id = match serde_json::from_value::<kernel::EntityId>(disk).unwrap() {
        kernel::EntityId::Face(f) => f,
        _ => unreachable!(),
    };
    // A 24-gon annulus from radius 1.5 to 2, 0.5 high, facing out.
    let annulus = 12.0 * (std::f64::consts::PI / 12.0).sin() * (4.0 - 2.25);
    let v = volume(m, &[disk_id]);
    assert!((v - annulus * 0.5).abs() < 1e-9, "{v}");
}

#[test]
fn a_profile_too_big_for_a_turn_is_refused_and_changes_nothing() {
    let mut d = doc();
    let path = [
        edge(&mut d, [0.0, 0.0, 0.0], [0.2, 0.0, 0.0]),
        edge(&mut d, [0.2, 0.0, 0.0], [0.2, 0.2, 0.0]),
    ];
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
    let (before, revision) = (counts(d.model()), d.revision());
    let err = run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
    assert!(err.1.contains("too big"), "{}", err.1);
    assert_eq!(counts(d.model()), before);
    assert_eq!(d.revision(), revision);
}

#[test]
fn a_circle_profile_gives_soft_sides_and_a_polygon_hard_ones() {
    for (kind, soft) in [("add_circle", 24), ("add_polygon", 0)] {
        let mut d = doc();
        let path = [
            edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
            edge(&mut d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]),
        ];
        let count = if kind == "add_circle" {
            "segments"
        } else {
            "sides"
        };
        let out = run(
            &mut d,
            json!({"type": kind, "center": [0, 0, 0], "radius": 0.3, count: 12, "normal": [1, 0, 0]}),
        )
        .unwrap();
        let face = created(&out, 'f')[0].clone();
        run(
            &mut d,
            json!({"type": "follow_me", "face": face, "path": path}),
        )
        .unwrap();
        d.scene().validate().unwrap();
        assert!(!exists(&d, &face), "the profile face is gone");
        let m = d.model();
        // Twelve sides on each of two segments, and two caps.
        assert_eq!(m.faces().count(), 26, "{kind}");
        // The corner between two straight path edges stays hard.
        assert_eq!(soft_edges(m), soft, "{kind}");
        assert!(volume(m, &[]) > 0.0, "{kind}");
    }
}

#[test]
fn a_path_that_is_another_face_s_loop_may_share_the_profile_s_edge() {
    let mut d = doc();
    // A ground face with a 135-degree corner at the origin.
    edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    edge(&mut d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]);
    edge(&mut d, [2.0, 2.0, 0.0], [-2.0, 2.0, 0.0]);
    edge(&mut d, [-2.0, 2.0, 0.0], [0.0, 0.0, 0.0]);
    // A triangle standing on the ground's slanted side, at the corner.
    edge(&mut d, [0.0, 0.0, 0.0], [0.0, 0.0, 0.3]);
    let out = run(
        &mut d,
        json!({"type": "add_edge", "start": [0, 0, 0.3], "end": [-0.3, 0.3, 0]}),
    )
    .unwrap();
    let face = created(&out, 'f')[0].clone();
    let ground = d
        .model()
        .faces()
        .find(|(_, f)| f.normal.z.abs() > 0.5)
        .map(|(_, f)| f.clone())
        .unwrap();
    let path: Vec<Value> = (0..ground.outer.len())
        .map(|i| {
            let (a, b) = (ground.outer[i], ground.outer[(i + 1) % ground.outer.len()]);
            json!(kernel::EntityId::Edge(
                d.model().edge_between(a, b).unwrap()
            ))
        })
        .collect();
    assert_eq!(path.len(), 5);
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    assert!(!exists(&d, &face));
    // Three sides on each of five segments, and the ground.
    assert_eq!(d.model().faces().count(), 16);
}

#[test]
fn refusals_change_nothing() {
    let mut d = doc();
    let a = edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let b = edge(&mut d, [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    let c = edge(&mut d, [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]);
    let tiny = edge(&mut d, [2.0, 0.0, 0.0], [2.0, 0.000005, 0.0]);
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [0.0, 0.1, 0.1], [1.0, 0.0, 0.0]);
    let side = d
        .model()
        .edges()
        .find(|(_, e)| {
            let [p, q] = e.v.map(|v| d.model().position(v).unwrap());
            p.x == 0.0 && q.x == 0.0
        })
        .map(|(e, _)| json!(kernel::EntityId::Edge(e)))
        .unwrap();
    let (before, revision) = (counts(d.model()), d.revision());
    let cases: Vec<(Value, ErrorKind, &str)> = vec![
        (json!([a, b, c]), ErrorKind::InvalidInput, "branch"),
        (json!([side]), ErrorKind::InvalidInput, "profile"),
        (json!([b, tiny]), ErrorKind::InvalidInput, "at least"),
        (json!([]), ErrorKind::InvalidInput, "path"),
        (json!([face]), ErrorKind::InvalidInput, "edges"),
        (json!(["e999"]), ErrorKind::NotFound, "no such"),
    ];
    for (path, kind, says) in cases {
        let err = run(
            &mut d,
            json!({"type": "follow_me", "face": face, "path": path}),
        )
        .unwrap_err();
        assert_eq!(err.0, kind, "{path}: {}", err.1);
        assert!(err.1.contains(says), "{path}: {}", err.1);
        assert_eq!(counts(d.model()), before);
        assert_eq!(d.revision(), revision);
    }
    let err = run(&mut d, json!({"type": "follow_me", "face": a, "path": [b]})).unwrap_err();
    assert_eq!(err.0, ErrorKind::InvalidInput);
    assert!(err.1.contains("face"), "{}", err.1);
    assert_eq!(d.revision(), revision);
}

#[test]
fn a_closed_path_out_of_one_plane_spreads_its_twist_over_split_sides() {
    let mut d = doc();
    let corners = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 4.0, 2.0],
        [0.0, 4.0, 4.0],
        [-2.0, 2.0, 1.0],
    ];
    let path: Vec<Value> = (0..5)
        .map(|i| edge(&mut d, corners[i], corners[(i + 1) % 5]))
        .collect();
    let face = rectangle(&mut d, [0.0, 0.2, 0.2], [0.0, 0.4, 0.4], [1.0, 0.0, 0.0]);
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    assert!(!exists(&d, &face));
    let m = d.model();
    // Every side of every segment is bent by the twist: two triangles each, split by a soft
    // diagonal.
    assert_eq!(m.faces().count(), 40);
    assert_eq!(soft_edges(m), 20);
    for (e, edge) in m.edges() {
        if edge.soft {
            assert!(m.faces_beside(e).is_some());
        }
    }
    assert!(volume(m, &[]) > 0.0);
}

#[test]
fn a_batch_can_refer_to_the_face_and_the_path_it_drew() {
    let mut d = doc();
    let commands: Vec<session::BatchCommand> = [
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [2, 0, 0]}),
        json!({"type": "add_edge", "start": [2, 0, 0], "end": [2, 2, 0]}),
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [0, 1, 1], "normal": [1, 0, 0]}),
        json!({"type": "follow_me", "face": "$2:f0", "path": ["$0:e*", "$1:e*"]}),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| session::BatchCommand::parse(v, i).unwrap())
    .collect();
    d.execute_batch(commands, true).unwrap();
    d.scene().validate().unwrap();
    assert_eq!(d.model().faces().count(), 10);
    assert!(closed_solid(d.model()));
}

fn material(d: &mut Document, color: [u8; 3]) -> String {
    let out = run(d, json!({"type": "add_material", "color": color})).unwrap();
    out["changes"]["materials"][0].as_str().unwrap().to_string()
}

#[test]
fn the_caps_keep_the_profile_s_paint() {
    let mut d = doc();
    let path = [
        edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        edge(&mut d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]),
    ];
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
    let (red, blue) = (material(&mut d, [200, 0, 0]), material(&mut d, [0, 0, 200]));
    run(
        &mut d,
        json!({"type": "paint", "ids": [face], "material": red, "side": "front"}),
    )
    .unwrap();
    run(
        &mut d,
        json!({"type": "paint", "ids": [face], "material": blue, "side": "back"}),
    )
    .unwrap();
    let id = |m: &str| serde_json::from_value::<kernel::MaterialId>(json!(m)).unwrap();
    let (red, blue) = (id(&red), id(&blue));
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    let painted: Vec<_> = m.faces().filter(|(_, f)| f.front.is_some()).collect();
    // The two caps, each square to its end of the path, the sides unpainted.
    assert_eq!(painted.len(), 2);
    for (_, f) in painted {
        assert_eq!((f.front, f.back), (Some(red), Some(blue)));
        assert!(f.normal.x.abs() > 0.99 || f.normal.y.abs() > 0.99);
    }
}

#[test]
fn a_profile_with_a_hole_gives_a_hollow_solid_with_open_caps() {
    let mut d = doc();
    let path = [
        edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        edge(&mut d, [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]),
    ];
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
    let inner = rectangle(
        &mut d,
        [0.0, 0.25, 0.25],
        [0.0, 0.75, 0.75],
        [1.0, 0.0, 0.0],
    );
    run(&mut d, json!({"type": "erase", "ids": [inner]})).unwrap();
    assert!(exists(&d, &face));
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    let m = d.model();
    // Four outer and four inner sides on each of two segments, and two caps.
    assert_eq!(m.faces().count(), 18);
    let holed: Vec<_> = m.faces().filter(|(_, f)| f.holes.len() == 1).collect();
    assert_eq!(holed.len(), 2, "both caps keep the hole");
    assert!(closed_solid(m));
    // The L of the outer square (width 1 along a centre line of 3) less that of the hole
    // (width 0.5 along 3), each as high as it is wide.
    let v = volume(m, &[]);
    assert!((v - (3.0 - 0.75)).abs() < 1e-9, "{v}");
}

#[test]
fn an_edge_the_profile_shares_with_a_neighbouring_face_survives() {
    let mut d = doc();
    // Ground behind the profile, its side along the profile's bottom edge.
    let ground = rectangle(&mut d, [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let path = [edge(&mut d, [0.0, 0.0, 0.0], [2.0, 0.0, 0.0])];
    let face = rectangle(&mut d, [0.0, 0.0, 0.0], [0.0, 1.0, 1.0], [1.0, 0.0, 0.0]);
    let at = |d: &Document, p: DVec3| {
        d.model()
            .vertices()
            .find(|(_, v)| v.pos.distance(p) < 1e-9)
            .unwrap()
            .0
    };
    let shared = d
        .model()
        .edge_between(at(&d, DVec3::ZERO), at(&d, DVec3::Y))
        .unwrap();
    run(
        &mut d,
        json!({"type": "follow_me", "face": face, "path": path}),
    )
    .unwrap();
    d.scene().validate().unwrap();
    assert!(!exists(&d, &face));
    assert!(exists(&d, &ground));
    let m = d.model();
    assert!(m.edge(shared).is_some(), "the shared edge survives");
    // The ground, four sides and two caps.
    assert_eq!(m.faces().count(), 7);
    assert!(volume(m, &[]) > 0.0);
}
