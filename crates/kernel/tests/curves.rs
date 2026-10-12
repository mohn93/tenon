use std::collections::BTreeSet;
use std::f64::consts::TAU;

use glam::{DAffine3, DVec3};
use kernel::{
    CurveId, CurveKind, DefinitionKind, EdgeId, EntityId, MeshId, Model, Scene, SceneData,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// The corners of a regular polygon with `n` sides around `center`, on the ground, the first
/// one along +X, counter-clockwise.
fn ring(center: DVec3, radius: f64, n: usize) -> Vec<DVec3> {
    (0..n)
        .map(|k| {
            let a = TAU * k as f64 / n as f64;
            center + p(a.cos(), a.sin(), 0.0) * radius
        })
        .collect()
}

/// The points of an arc around `center` from angle 0 sweeping `angle` radians, `n` segments.
fn arc(center: DVec3, radius: f64, angle: f64, n: usize) -> Vec<DVec3> {
    (0..=n)
        .map(|k| {
            let a = angle * k as f64 / n as f64;
            center + p(a.cos(), a.sin(), 0.0) * radius
        })
        .collect()
}

/// The curves of a mesh's edges.
fn curves(m: &Model) -> BTreeSet<CurveId> {
    m.edges().filter_map(|(_, e)| e.curve).collect()
}

fn only_curve(m: &Model) -> CurveId {
    let all = curves(m);
    assert_eq!(all.len(), 1, "{all:?}");
    *all.iter().next().unwrap()
}

fn edge_at(m: &Model, a: DVec3, b: DVec3) -> EdgeId {
    let v = |q: DVec3| {
        m.vertices()
            .find(|(_, v)| v.pos.distance(q) < 1e-9)
            .map(|(id, _)| id)
            .unwrap()
    };
    m.edge_between(v(a), v(b)).unwrap()
}

fn close(a: DVec3, b: DVec3) -> bool {
    a.distance(b) < 1e-9
}

#[test]
fn a_drawn_circle_is_one_curve_whose_shape_comes_from_its_edges() {
    let mut m = Model::new();
    let points = ring(p(1.0, 2.0, 0.0), 2.0, 24);
    let c = m.add_curve(&points, true).unwrap();
    m.validate().unwrap();
    assert_eq!((m.edges().count(), m.faces().count()), (24, 1));
    let id = only_curve(&m);
    assert!(m.edges().all(|(_, e)| e.curve == Some(id) && !e.soft));

    let curve = m.curve(id).unwrap();
    assert_eq!(curve.kind(), CurveKind::Circle);
    assert!(curve.closed);
    assert_eq!(
        (curve.edges.len(), curve.vertices.len(), curve.segments),
        (24, 24, 24)
    );
    let circle = curve.circle.unwrap();
    assert!(close(circle.center, p(1.0, 2.0, 0.0)), "{circle:?}");
    assert!((circle.radius - 2.0).abs() < 1e-9);
    assert_eq!(circle.normal, DVec3::Z);
    assert_eq!(circle.angle, TAU);
    let side = points[0].distance(points[1]);
    assert!((curve.length - 24.0 * side).abs() < 1e-9);
    // It starts at the first corner drawn and runs counter-clockwise.
    assert!(close(m.position(curve.vertices[0]).unwrap(), points[0]));
    assert!(close(m.position(curve.vertices[1]).unwrap(), points[1]));
    let e0 = curve.edges[0];
    assert_eq!(m.curve_of(e0).unwrap(), curve);

    // Undo and redo restore the curve exactly.
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, Model::new());
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn a_circle_drawn_clockwise_reports_an_upward_normal() {
    let mut m = Model::new();
    let mut points = ring(DVec3::ZERO, 1.0, 12);
    points.reverse();
    m.add_curve(&points, true).unwrap();
    let curve = m.curve(only_curve(&m)).unwrap();
    assert_eq!(curve.circle.unwrap().normal, DVec3::Z);
    let at = |i: usize| m.position(curve.vertices[i]).unwrap();
    // Counter-clockwise around +Z: the second vertex is ahead of the first.
    assert!(at(0).cross(at(1)).z > 0.0);
}

#[test]
fn an_open_polyline_is_an_arc_and_a_straight_one_is_no_curve() {
    let mut m = Model::new();
    m.add_curve(&arc(DVec3::ZERO, 3.0, TAU / 4.0, 6), false)
        .unwrap();
    m.validate().unwrap();
    let curve = m.curve(only_curve(&m)).unwrap();
    assert_eq!(curve.kind(), CurveKind::Arc);
    assert_eq!((curve.edges.len(), curve.segments), (6, 6));
    let circle = curve.circle.unwrap();
    assert!(close(circle.center, DVec3::ZERO));
    assert!((circle.radius - 3.0).abs() < 1e-9);
    assert!((circle.angle - TAU / 4.0).abs() < 1e-12);
    assert!(close(
        m.position(curve.vertices[0]).unwrap(),
        p(3.0, 0.0, 0.0)
    ));

    let mut straight = Model::new();
    straight
        .add_curve(&[DVec3::ZERO, p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)], false)
        .unwrap();
    straight.validate().unwrap();
    assert!(curves(&straight).is_empty());
}

#[test]
fn split_curve_edges_stay_on_the_curve_and_heal_back() {
    let mut m = Model::new();
    let points = ring(DVec3::ZERO, 1.0, 24);
    m.add_curve(&points, true).unwrap();
    let id = only_curve(&m);
    // A line across the circle splits two of its edges in their middles.
    let a = (points[3] + points[4]) / 2.0;
    let b = (points[15] + points[16]) / 2.0;
    let line = m.add_edge(a, b).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    let curve = m.curve(id).unwrap();
    assert_eq!((curve.edges.len(), curve.segments), (26, 24));
    assert!((curve.circle.unwrap().radius - 1.0).abs() < 1e-9);
    let across = line
        .summary()
        .created
        .iter()
        .find_map(|e| match e {
            EntityId::Edge(e) if m.edge(*e).is_some_and(|x| x.curve.is_none()) => Some(*e),
            _ => None,
        })
        .unwrap();

    // Erasing the line heals the split edges back into the circle.
    m.erase(&[across], &[]);
    m.validate().unwrap();
    assert_eq!(m.edges().count(), 24);
    assert_eq!(m.curve(id).unwrap().edges.len(), 24);
}

#[test]
fn erasing_curve_edges_leaves_arcs_and_a_lone_edge_is_no_curve() {
    let mut m = Model::new();
    let points = ring(DVec3::ZERO, 1.0, 12);
    m.add_curve(&points, true).unwrap();
    let id = only_curve(&m);

    let first = m.erase(&[edge_at(&m, points[0], points[1])], &[]);
    m.validate().unwrap();
    let curve = m.curve(id).unwrap();
    assert_eq!(curve.kind(), CurveKind::Arc);
    assert_eq!(curve.segments, 11);
    assert!((curve.circle.unwrap().angle - TAU * 11.0 / 12.0).abs() < 1e-12);
    // The face went with the edge; the rest stays one curve, untouched.
    assert!(first.summary().modified.is_empty());

    // Erasing an edge in the middle splits it in two: the older half keeps the id.
    let second = m.erase(&[edge_at(&m, points[6], points[7])], &[]);
    m.validate().unwrap();
    let all = curves(&m);
    assert_eq!(all.len(), 2);
    assert!(all.contains(&id));
    let other = *all.iter().find(|c| **c != id).unwrap();
    assert!(other.0 > id.0, "the new curve gets a new id");
    let lengths: BTreeSet<usize> = all.iter().map(|c| m.curve(*c).unwrap().segments).collect();
    assert_eq!(lengths, BTreeSet::from([5]));
    assert_eq!(
        second.summary().modified.len(),
        5,
        "the renamed half is modified"
    );

    // Erase down to one edge: it is no curve any more.
    let rest: Vec<EdgeId> = m.curve(other).unwrap().edges[1..].to_vec();
    m.erase(&rest, &[]);
    m.validate().unwrap();
    assert!(!curves(&m).contains(&other));
    assert_eq!(m.edges().filter(|(_, e)| e.curve.is_none()).count(), 1);
}

#[test]
fn moved_turned_and_copied_curves_follow_their_edges() {
    let mut m = Model::new();
    m.add_curve(&ring(DVec3::ZERO, 1.0, 8), true).unwrap();
    let id = only_curve(&m);
    let all: BTreeSet<_> = m.vertices().map(|(v, _)| v).collect();
    m.move_vertices(&all, p(5.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    assert!(close(
        m.curve(id).unwrap().circle.unwrap().center,
        p(5.0, 0.0, 0.0)
    ));

    // Stood up on its side, its normal turns too.
    let turn = DAffine3::from_translation(p(5.0, 0.0, 0.0))
        * DAffine3::from_rotation_x(TAU / 4.0)
        * DAffine3::from_translation(p(-5.0, 0.0, 0.0));
    m.transform_vertices(&all, turn).unwrap();
    let circle = m.curve(id).unwrap().circle.unwrap();
    assert!(close(circle.normal, DVec3::Y), "{circle:?}");

    let edges: Vec<EdgeId> = m.edges().map(|(e, _)| e).collect();
    let faces: Vec<_> = m.faces().map(|(f, _)| f).collect();
    m.copy(&edges, &faces, p(0.0, 3.0, 0.0), 2).unwrap();
    m.validate().unwrap();
    let ids = curves(&m);
    assert_eq!(ids.len(), 3, "each copy is a curve of its own");
    let centers: Vec<DVec3> = ids
        .iter()
        .map(|c| m.curve(*c).unwrap().circle.unwrap().center)
        .collect();
    assert!(close(centers[1], p(5.0, 3.0, 0.0)) && close(centers[2], p(5.0, 6.0, 0.0)));
}

#[test]
fn curves_go_into_groups_and_come_out_of_explode_with_new_ids() {
    let mut m = Model::new();
    m.add_curve(&ring(DVec3::ZERO, 1.0, 12), true).unwrap();
    let id = only_curve(&m);
    let mut s = Scene::from_root(m);
    let edges: Vec<EdgeId> = s.root().edges().map(|(e, _)| e).collect();
    let faces: Vec<_> = s.root().faces().map(|(f, _)| f).collect();
    let (i, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    s.validate().unwrap();
    let d = s.root().instance(i).unwrap().definition;
    let inner = only_curve(&s.definition(d).unwrap().mesh);
    assert_ne!(inner, id);
    assert_eq!(
        s.definition(d).unwrap().mesh.curve(inner).unwrap().kind(),
        CurveKind::Circle
    );

    s.explode(MeshId::Root, i).unwrap();
    s.validate().unwrap();
    let out = only_curve(s.root());
    assert!(out != id && out != inner);
    assert_eq!(s.root().curve(out).unwrap().segments, 12);
}

#[test]
fn curves_and_soft_flags_are_saved_and_checked_on_load() {
    let mut m = Model::new();
    m.add_curve(&ring(DVec3::ZERO, 1.0, 6), true).unwrap();
    let s = Scene::from_root(m);
    let data = s.to_data();
    let json = serde_json::to_value(&data).unwrap();
    let edges = json["root"]["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .all(|e| e["curve"].is_u64() && e.get("soft").is_none())
    );
    let back = Scene::from_data(&data).unwrap();
    assert_eq!(back, s);

    // Files written before curves have neither field.
    let mut old = json.clone();
    for e in old["root"]["edges"].as_array_mut().unwrap() {
        e.as_object_mut().unwrap().remove("curve");
    }
    let old: SceneData = serde_json::from_value(old).unwrap();
    assert!(curves(Scene::from_data(&old).unwrap().root()).is_empty());

    // A curve that branches, or a soft edge with no faces, is refused.
    let mut bad = json.clone();
    bad["root"]["edges"][0]["curve"] = serde_json::json!(999);
    let bad: SceneData = serde_json::from_value(bad).unwrap();
    let err = Scene::from_data(&bad).unwrap_err();
    assert!(err.contains("curve c999"), "{err}");

    let mut lone = Model::new();
    lone.add_edge(DVec3::ZERO, DVec3::X).unwrap();
    let mut soft = serde_json::to_value(Scene::from_root(lone).to_data()).unwrap();
    soft["root"]["edges"][0]["soft"] = serde_json::json!(true);
    let soft: SceneData = serde_json::from_value(soft).unwrap();
    let err = Scene::from_data(&soft).unwrap_err();
    assert!(err.contains("soft edge"), "{err}");
}
