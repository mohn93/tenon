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
    let c = m.add_curve(&points, true, false).unwrap();
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
    m.add_curve(&points, true, false).unwrap();
    let curve = m.curve(only_curve(&m)).unwrap();
    assert_eq!(curve.circle.unwrap().normal, DVec3::Z);
    let at = |i: usize| m.position(curve.vertices[i]).unwrap();
    // Counter-clockwise around +Z: the second vertex is ahead of the first.
    assert!(at(0).cross(at(1)).z > 0.0);
}

#[test]
fn an_open_polyline_is_an_arc_and_a_straight_one_is_no_curve() {
    let mut m = Model::new();
    m.add_curve(&arc(DVec3::ZERO, 3.0, TAU / 4.0, 6), false, false)
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
        .add_curve(
            &[DVec3::ZERO, p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)],
            false,
            false,
        )
        .unwrap();
    straight.validate().unwrap();
    assert!(curves(&straight).is_empty());
}

#[test]
fn split_curve_edges_stay_on_the_curve_and_heal_back() {
    let mut m = Model::new();
    let points = ring(DVec3::ZERO, 1.0, 24);
    m.add_curve(&points, true, false).unwrap();
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
    m.add_curve(&points, true, false).unwrap();
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
    m.add_curve(&ring(DVec3::ZERO, 1.0, 8), true, false)
        .unwrap();
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
    m.add_curve(&ring(DVec3::ZERO, 1.0, 12), true, false)
        .unwrap();
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
    m.add_curve(&ring(DVec3::ZERO, 1.0, 6), true, false)
        .unwrap();
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

/// The face a change created.
fn created_face(c: &kernel::ChangeSet) -> kernel::FaceId {
    c.summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap()
}

fn soft_edges(m: &Model) -> Vec<EdgeId> {
    m.edges()
        .filter(|(_, e)| e.soft)
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_pulled_circle_has_soft_sides_and_a_curve_on_top() {
    let mut m = Model::new();
    let c = m
        .add_curve(&ring(DVec3::ZERO, 1.0, 12), true, false)
        .unwrap();
    let base = only_curve(&m);
    let pulled = m.push_pull(created_face(&c), 2.0).unwrap();
    m.validate().unwrap();
    assert_eq!((m.edges().count(), m.faces().count()), (36, 14));

    // The twelve swept edges are soft; the rims are not.
    let soft = soft_edges(&m);
    assert_eq!(soft.len(), 12);
    for e in &soft {
        let [a, b] = m.edge(*e).unwrap().v;
        let d = m.position(b).unwrap() - m.position(a).unwrap();
        assert!(d.x.abs() < 1e-12 && d.y.abs() < 1e-12, "a swept edge");
        assert!(m.edge(*e).unwrap().curve.is_none());
    }
    // The bottom keeps its curve; the top is a new one, 2 m up.
    let all = curves(&m);
    assert_eq!(all.len(), 2);
    assert!(all.contains(&base));
    let top = *all.iter().find(|c| **c != base).unwrap();
    let circle = m.curve(top).unwrap().circle.unwrap();
    assert!(close(circle.center, p(0.0, 0.0, 2.0)));
    assert_eq!(m.curve(top).unwrap().segments, 12);

    // Undo is exact.
    let after = m.clone();
    m.apply(&pulled.inverse());
    assert!(soft_edges(&m).is_empty());
    m.apply(&pulled);
    assert_eq!(m, after);

    // Pulling the top again stretches the sides; nothing changes about the curves.
    let top_face = m
        .faces()
        .find(|(_, f)| f.normal.z > 0.5 && m.position(f.outer[0]).unwrap().z > 1.0)
        .map(|(id, _)| id)
        .unwrap();
    m.push_pull(top_face, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!((soft_edges(&m).len(), curves(&m).len()), (12, 2));
}

#[test]
fn a_soft_edge_left_with_one_face_hardens() {
    let mut m = Model::new();
    let c = m
        .add_curve(&ring(DVec3::ZERO, 1.0, 8), true, false)
        .unwrap();
    m.push_pull(created_face(&c), 1.0).unwrap();
    let side = m
        .faces()
        .find(|(_, f)| f.normal.z.abs() < 0.5)
        .map(|(id, _)| id)
        .unwrap();
    let erased = m.erase(&[], &[side]);
    m.validate().unwrap();
    assert_eq!(
        soft_edges(&m).len(),
        6,
        "the two edges beside the hole harden"
    );
    assert_eq!(erased.summary().modified.len(), 2);
}

#[test]
fn an_arc_closed_by_a_line_pulls_up_with_hard_ends() {
    let mut m = Model::new();
    // A half circle from (1, 0) over the top to (-1, 0), closed by its diameter.
    m.add_curve(&arc(DVec3::ZERO, 1.0, TAU / 2.0, 6), false, false)
        .unwrap();
    let c = m.add_edge(p(-1.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    m.validate().unwrap();
    let face = created_face(&c);
    m.push_pull(face, 1.0).unwrap();
    m.validate().unwrap();
    // Five soft edges between the six arc sides; the two at the arc's ends stay hard.
    assert_eq!(soft_edges(&m).len(), 5);
    let ends = [p(1.0, 0.0, 0.0), p(-1.0, 0.0, 0.0)];
    for e in soft_edges(&m) {
        let a = m.position(m.edge(e).unwrap().v[0]).unwrap();
        assert!(ends.iter().all(|q| (a - *q).truncate().length() > 1e-9));
    }
    let top: Vec<_> = curves(&m)
        .iter()
        .map(|c| m.curve(*c).unwrap())
        .filter(|c| c.circle.unwrap().center.z > 0.5)
        .collect();
    assert_eq!(top.len(), 1);
    assert!((top[0].circle.unwrap().angle - TAU / 2.0).abs() < 1e-9);
}

#[test]
fn a_circle_pushed_into_a_box_makes_a_soft_walled_hole() {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            p(-2.0, -2.0, 0.0),
            p(2.0, -2.0, 0.0),
            p(2.0, 2.0, 0.0),
            p(-2.0, 2.0, 0.0),
        ])
        .unwrap();
    m.push_pull(created_face(&c), 2.0).unwrap();
    let mut top_ring = ring(DVec3::ZERO, 1.0, 8);
    for q in &mut top_ring {
        q.z = 2.0;
    }
    let disk = m.add_curve(&top_ring, true, false).unwrap();
    let disk_face = disk
        .summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) if m.face(*f).unwrap().outer.len() == 8 => Some(*f),
            _ => None,
        })
        .unwrap();
    m.push_pull(disk_face, -1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(soft_edges(&m).len(), 8);
    assert_eq!(curves(&m).len(), 2, "the rim and the floor of the hole");
}

#[test]
fn a_straight_polyline_spends_no_curve_id() {
    let mut m = Model::new();
    let c = m
        .add_curve(
            &[p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)],
            false,
            false,
        )
        .unwrap();
    assert!(curves(&m).is_empty());
    // Three vertices and two edges took ids 0 to 4; the next edit starts at 5.
    assert_eq!(c.summary().created.len(), 5);
    let next = m.add_edge(p(0.0, 5.0, 0.0), p(1.0, 5.0, 0.0)).unwrap();
    let ids: Vec<u64> = next
        .summary()
        .created
        .iter()
        .map(|id| match id {
            EntityId::Vertex(v) => v.0,
            EntityId::Edge(e) => e.0,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(ids.iter().min(), Some(&5));
}

/// A hexagon's corners: radius 1 around the origin, the first along +X.
fn hexagon() -> Vec<DVec3> {
    ring(DVec3::ZERO, 1.0, 6)
}

#[test]
fn a_polygon_curve_reads_as_a_polygon() {
    let mut m = Model::new();
    m.add_curve(&hexagon(), true, true).unwrap();
    m.validate().unwrap();
    let curve = m.curve_of(m.edges().next().unwrap().0).unwrap();
    assert_eq!(curve.kind(), CurveKind::Polygon);
    assert!(curve.polygon);
    assert_eq!(curve.segments, 6);
    assert!((curve.circle.unwrap().radius - 1.0).abs() < 1e-9);
    assert!(m.edges().all(|(_, e)| e.polygon && !e.soft));

    let mut c = Model::new();
    c.add_curve(&hexagon(), true, false).unwrap();
    let circle = c.curve_of(c.edges().next().unwrap().0).unwrap();
    assert_eq!(circle.kind(), CurveKind::Circle);
    assert!(!circle.polygon);
    assert!(c.edges().all(|(_, e)| !e.polygon));
}

#[test]
fn a_pulled_polygon_has_hard_sides() {
    let mut m = Model::new();
    let c = m.add_curve(&hexagon(), true, true).unwrap();
    m.push_pull(created_face(&c), 1.0).unwrap();
    m.validate().unwrap();
    assert!(soft_edges(&m).is_empty());
    let all = curves(&m);
    assert_eq!(all.len(), 2);
    let top: Vec<_> = all
        .iter()
        .map(|c| m.curve(*c).unwrap())
        .filter(|c| c.circle.unwrap().center.z > 0.5)
        .collect();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].kind(), CurveKind::Polygon);
    assert_eq!(top[0].segments, 6);
    let kinds: Vec<_> = all.iter().map(|c| m.curve(*c).unwrap().kind()).collect();
    assert_eq!(kinds, [CurveKind::Polygon; 2]);

    let mut circle = Model::new();
    let c = circle.add_curve(&hexagon(), true, false).unwrap();
    circle.push_pull(created_face(&c), 1.0).unwrap();
    circle.validate().unwrap();
    assert_eq!(soft_edges(&circle).len(), 6);
}

/// The kinds of a mesh's curves, in id order.
fn kinds(m: &Model) -> Vec<CurveKind> {
    curves(m)
        .iter()
        .map(|c| m.curve(*c).unwrap().kind())
        .collect()
}

#[test]
fn copies_splits_and_files_keep_the_polygon_mark() {
    let pts = hexagon();
    let mut m = Model::new();
    m.add_curve(&pts, true, true).unwrap();
    let id = only_curve(&m);
    let edges: Vec<EdgeId> = m.edges().map(|(e, _)| e).collect();
    let faces: Vec<_> = m.faces().map(|(f, _)| f).collect();
    m.copy(&edges, &faces, p(3.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    let all = curves(&m);
    assert_eq!(all.len(), 2);
    let copy = *all.iter().find(|c| **c != id).unwrap();
    assert_eq!(m.curve(copy).unwrap().kind(), CurveKind::Polygon);
    assert!(close(
        m.curve(copy).unwrap().circle.unwrap().center,
        p(3.0, 0.0, 0.0)
    ));

    // A line from the middle of a side to the center splits that side.
    m.add_edge((pts[0] + pts[1]) / 2.0, DVec3::ZERO).unwrap();
    m.validate().unwrap();
    let curve = m.curve(id).unwrap();
    assert_eq!((curve.edges.len(), curve.segments), (7, 6));
    assert_eq!(curve.kind(), CurveKind::Polygon);

    // Saved and opened again.
    let s = Scene::from_root(m.clone());
    let back = Scene::from_data(&s.to_data()).unwrap();
    assert_eq!(back, s);
    assert_eq!(kinds(back.root()), [CurveKind::Polygon; 2]);

    // Grouped and exploded.
    let mut g = Scene::from_root(m.clone());
    let edges: Vec<EdgeId> = g.root().edges().map(|(e, _)| e).collect();
    let faces: Vec<_> = g.root().faces().map(|(f, _)| f).collect();
    let (i, _) = g
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    g.validate().unwrap();
    let d = g.root().instance(i).unwrap().definition;
    assert_eq!(
        kinds(&g.definition(d).unwrap().mesh),
        [CurveKind::Polygon; 2]
    );
    g.explode(MeshId::Root, i).unwrap();
    g.validate().unwrap();
    assert_eq!(kinds(g.root()), [CurveKind::Polygon; 2]);

    // Opened up by erasing a side, it reads as a curve.
    m.erase(&[edge_at(&m, pts[2], pts[3])], &[]);
    m.validate().unwrap();
    let opened = m.curve(id).unwrap();
    assert!(opened.polygon);
    assert_eq!(opened.kind(), CurveKind::Curve);
}

#[test]
fn polygon_flags_agree_along_a_curve() {
    let mut m = Model::new();
    m.add_curve(&hexagon(), true, true).unwrap();
    let json = serde_json::to_value(Scene::from_root(m).to_data()).unwrap();
    let edges = json["root"]["edges"].as_array().unwrap();
    assert!(edges.iter().all(|e| e["polygon"] == true));

    let mut mixed = json.clone();
    mixed["root"]["edges"][0]
        .as_object_mut()
        .unwrap()
        .remove("polygon");
    let mixed: SceneData = serde_json::from_value(mixed).unwrap();
    let err = Scene::from_data(&mixed).unwrap_err();
    assert!(err.contains("polygon"), "{err}");

    // A plain edge is never a polygon side.
    let mut lone = Model::new();
    lone.add_edge(DVec3::ZERO, DVec3::X).unwrap();
    let mut plain = serde_json::to_value(Scene::from_root(lone).to_data()).unwrap();
    plain["root"]["edges"][0]["polygon"] = serde_json::json!(true);
    let plain: SceneData = serde_json::from_value(plain).unwrap();
    let err = Scene::from_data(&plain).unwrap_err();
    assert!(err.contains("polygon"), "{err}");
}
