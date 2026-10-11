use std::collections::BTreeSet;

use glam::DVec3;
use kernel::{EntityId, FaceId, KernelError, Model, VertexId};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn square(m: &mut Model, x0: f64, y0: f64, size: f64) -> FaceId {
    let c = m
        .add_polygon(&[
            p(x0, y0, 0.0),
            p(x0 + size, y0, 0.0),
            p(x0 + size, y0 + size, 0.0),
            p(x0, y0 + size, 0.0),
        ])
        .unwrap();
    c.summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap()
}

/// A 2 m cube on the ground, built with push/pull.
fn cube() -> Model {
    let mut m = Model::new();
    let base = square(&mut m, 0.0, 0.0, 2.0);
    m.push_pull(base, 2.0).unwrap();
    m
}

fn vertices_where(m: &Model, f: impl Fn(DVec3) -> bool) -> BTreeSet<VertexId> {
    m.vertices()
        .filter(|(_, v)| f(v.pos))
        .map(|(id, _)| id)
        .collect()
}

fn face_with_normal(m: &Model, n: DVec3) -> FaceId {
    m.faces()
        .find(|(_, f)| f.normal.distance(n) < 1e-9)
        .map(|(id, _)| id)
        .unwrap()
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

#[test]
fn moving_every_vertex_moves_a_solid_rigidly() {
    let mut m = cube();
    let before: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    let all = vertices_where(&m, |_| true);
    let c = m.move_vertices(&all, p(1.0, 2.0, 0.5)).unwrap();
    m.validate().unwrap();
    let after: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    assert_eq!(before, after);
    assert!(m.vertices().any(|(_, v)| v.pos == p(3.0, 4.0, 2.5)));
    let s = c.summary();
    assert_eq!(s.modified.len(), 8);
    assert!(s.created.is_empty() && s.deleted.is_empty());
}

#[test]
fn moving_a_top_edge_out_tilts_the_face_below_it() {
    let mut m = cube();
    let front = face_with_normal(&m, -DVec3::Y);
    let side = face_with_normal(&m, DVec3::X);
    let top_front = vertices_where(&m, |q| q.y == 0.0 && q.z == 2.0);
    assert_eq!(top_front.len(), 2);
    m.move_vertices(&top_front, p(0.0, -1.0, 0.0)).unwrap();
    m.validate().unwrap();
    let n = m.face(front).unwrap().normal;
    assert!(n.y < -0.5 && n.z < -0.1, "front face leans out: {n}");
    assert_eq!(
        m.face(side).unwrap().normal,
        DVec3::X,
        "side faces stay in their planes"
    );
    let top = face_with_normal(&m, DVec3::Z);
    assert!((m.area(top).unwrap() - 6.0).abs() < 1e-9);
}

#[test]
fn moving_a_corner_out_of_its_plane_is_refused() {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 2.0);
    let before = m.clone();
    let corner = vertices_where(&m, |q| q == p(0.0, 0.0, 0.0));
    assert_eq!(
        m.move_vertices(&corner, p(0.0, 0.0, 1.0)),
        Err(KernelError::Bend(f))
    );
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn moves_that_collapse_or_overlap_are_refused() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let before = m.clone();
    let corner = vertices_where(&m, |q| q == p(0.0, 0.0, 0.0));
    // Onto the neighbouring corner: the edge between them collapses.
    assert_eq!(
        m.move_vertices(&corner, p(2.0, 0.0, 0.0)),
        Err(KernelError::Squash)
    );
    // Across the square: the face turns inside out.
    assert_eq!(
        m.move_vertices(&corner, p(3.0, 3.0, 0.0)),
        Err(KernelError::Squash)
    );
    assert_eq!(
        m.move_vertices(&corner, p(0.0, 0.0, 0.0)),
        Err(KernelError::ZeroDistance)
    );
    assert_eq!(m, before);
}

#[test]
fn move_undoes_and_redoes_exactly() {
    let mut m = cube();
    let before = m.clone();
    let top_front = vertices_where(&m, |q| q.y == 0.0 && q.z == 2.0);
    let c = m.move_vertices(&top_front, p(0.0, -1.0, 0.0)).unwrap();
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn copying_a_face_keeps_its_orientation() {
    let mut m = cube();
    let bottom = face_with_normal(&m, -DVec3::Z);
    m.copy(&[], &[bottom], p(5.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (12, 16, 7));
    let copies: Vec<_> = m
        .faces()
        .filter(|(_, f)| f.outer.iter().all(|v| m.position(*v).unwrap().x >= 5.0))
        .collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].1.normal, -DVec3::Z);
}

#[test]
fn array_copies_make_count_copies() {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 1.0);
    let before = m.clone();
    let c = m.copy(&[], &[f], p(2.0, 0.0, 0.0), 3).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (16, 16, 4));
    assert!(m.vertices().any(|(_, v)| v.pos == p(7.0, 1.0, 0.0)));
    m.apply(&c.inverse());
    assert_eq!(m, before);
}

#[test]
fn copies_that_touch_merge_with_the_original() {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 2.0);
    m.copy(&[], &[f], p(2.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (6, 7, 2));
}

#[test]
fn copying_loose_edges_copies_only_edges() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)).unwrap();
    let e = m.edges().next().unwrap().0;
    m.copy(&[e], &[], p(0.0, 1.0, 0.0), 2).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (6, 3, 0));
    assert_eq!(
        m.copy(&[e], &[], DVec3::ZERO, 1),
        Err(KernelError::ZeroDistance)
    );
}

#[test]
fn moves_that_make_a_face_cross_itself_are_refused() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 2.0);
    let before = m.clone();
    let corner = vertices_where(&m, |q| q == p(0.0, 0.0, 0.0));
    assert_eq!(
        m.move_vertices(&corner, p(2.5, 1.0, 0.0)),
        Err(KernelError::Squash)
    );
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn moving_a_hole_out_of_its_face_is_refused() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.0, 1.0, 1.0);
    let before = m.clone();
    let hole = vertices_where(&m, |q| q.x >= 1.0 && q.x <= 2.0 && q.y >= 1.0 && q.y <= 2.0);
    assert_eq!(hole.len(), 4);
    assert_eq!(
        m.move_vertices(&hole, p(5.0, 0.0, 0.0)),
        Err(KernelError::Squash)
    );
    assert_eq!(m, before);
}
