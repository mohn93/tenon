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

#[test]
fn copying_a_face_with_an_opening_keeps_the_opening() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    let inner = square(&mut m, 1.5, 1.5, 1.0);
    m.erase(&[], &[inner]);
    m.validate().unwrap();
    let f = m.faces().next().map(|(id, _)| id).unwrap();
    assert_eq!(m.faces().count(), 1);
    assert_eq!(m.face(f).unwrap().holes.len(), 1);
    let before = m.clone();
    let c = m.copy(&[], &[f], p(10.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    let copy = m
        .faces()
        .find(|(id, _)| *id != f)
        .map(|(_, face)| face.clone())
        .unwrap();
    assert_eq!(copy.holes.len(), 1);
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn copying_a_face_with_a_filled_hole_does_not_remove_faces() {
    let mut m = Model::new();
    let outer = square(&mut m, 0.0, 0.0, 4.0);
    square(&mut m, 1.5, 1.5, 1.0);
    assert_eq!(m.faces().count(), 2);
    m.copy(&[], &[outer], p(10.0, 0.0, 0.0), 2).unwrap();
    m.validate().unwrap();
    // The hole copies close into faces, as the original's hole is filled.
    assert_eq!(m.faces().count(), 6);
}

#[test]
fn a_face_copied_onto_an_empty_outline_fills_it() {
    let mut m = cube();
    let top = face_with_normal(&m, DVec3::Z);
    m.erase(&[], &[top]);
    let bottom = face_with_normal(&m, -DVec3::Z);
    m.copy(&[], &[bottom], p(0.0, 0.0, 2.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    let lid = m
        .faces()
        .find(|(_, f)| f.outer.iter().all(|v| m.position(*v).unwrap().z > 1.0))
        .map(|(_, f)| f.normal);
    assert_eq!(
        lid,
        Some(-DVec3::Z),
        "the copy keeps the bottom's orientation"
    );
}

#[test]
fn copying_an_open_cube_adds_no_lid() {
    let mut m = cube();
    let top = face_with_normal(&m, DVec3::Z);
    m.erase(&[], &[top]);
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    m.copy(&[], &faces, p(10.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 10);
}

fn copy_next_to_parent_edge(a: DVec3, b: DVec3) {
    let mut m = Model::new();
    let f = square(&mut m, 0.0, 0.0, 1.0);
    m.add_edge(a, b).unwrap();
    m.copy(&[], &[f], p(10.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    for (_, face) in m.faces() {
        assert_eq!(face.normal, face_normal_of_first(&m));
    }
}

fn face_normal_of_first(m: &Model) -> DVec3 {
    m.faces().next().map(|(_, f)| f.normal).unwrap()
}

#[test]
fn a_copy_touched_by_a_parent_edge_keeps_its_face() {
    copy_next_to_parent_edge(p(10.5, -1.0, 0.0), p(10.5, 0.0, 0.0));
}

#[test]
fn a_copy_with_a_parent_edge_along_its_side_keeps_its_face() {
    copy_next_to_parent_edge(p(10.5, 0.0, 0.0), p(12.0, 0.0, 0.0));
}

#[test]
fn a_copied_opening_stays_open_when_a_parent_edge_splits_it() {
    for (a, b) in [
        (p(12.0, 1.5, 0.0), p(12.0, 2.0, 0.0)),
        (p(12.0, 1.5, 0.0), p(13.0, 1.5, 0.0)),
        (p(11.0, 1.5, 0.0), p(11.6, 1.5, 0.0)),
    ] {
        let mut m = Model::new();
        let outer = square(&mut m, 0.0, 0.0, 4.0);
        let inner = square(&mut m, 1.5, 1.5, 1.0);
        m.erase(&[], &[inner]);
        m.add_edge(a, b).unwrap();
        m.copy(&[], &[outer], p(10.0, 0.0, 0.0), 1).unwrap();
        m.validate().unwrap();
        assert_eq!(m.faces().count(), 2, "parent edge {a} to {b}");
        assert!(m.faces().all(|(_, f)| f.holes.len() == 1));
    }
}

/// The face ids of the model that the closure picks by the position of their first vertex.
fn faces_at(m: &Model, f: impl Fn(DVec3) -> bool) -> Vec<FaceId> {
    m.faces()
        .filter(|(_, face)| f(m.position(face.outer[0]).unwrap()))
        .map(|(id, _)| id)
        .collect()
}

/// A 4 m square with a 2 m opening and a 1 m island face inside the opening.
fn island_in_opening() -> Model {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 4.0);
    let hole = square(&mut m, 1.0, 1.0, 2.0);
    m.erase(&[], &[hole]);
    square(&mut m, 1.5, 1.5, 1.0);
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    m
}

#[test]
fn an_island_inside_a_copied_opening_is_copied() {
    let mut m = island_in_opening();
    let normal = face_normal_of_first(&m);
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    m.copy(&[], &faces, p(10.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 4);
    assert!(m.faces().all(|(_, f)| f.normal == normal));
    let copies = faces_at(&m, |q| q.x > 5.0);
    assert_eq!(copies.len(), 2);
    let holes: Vec<usize> = copies
        .iter()
        .map(|id| m.face(*id).unwrap().holes.len())
        .collect();
    assert!(holes.contains(&1) && holes.contains(&0), "{holes:?}");
}

#[test]
fn a_washer_inside_a_copied_washer_is_copied() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 10.0);
    let a = square(&mut m, 2.0, 2.0, 6.0);
    m.erase(&[], &[a]);
    square(&mut m, 3.0, 3.0, 4.0);
    let b = square(&mut m, 4.0, 4.0, 2.0);
    m.erase(&[], &[b]);
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    m.copy(&[], &faces, p(20.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 4);
    assert!(m.faces().all(|(_, f)| f.holes.len() == 1));
    let copies = faces_at(&m, |q| q.x > 15.0);
    assert_eq!(copies.len(), 2);
}

#[test]
fn an_island_split_by_a_parent_edge_is_copied() {
    let mut m = island_in_opening();
    let normal = face_normal_of_first(&m);
    m.add_edge(p(12.0, 1.5, 0.0), p(12.0, 2.5, 0.0)).unwrap();
    let faces: Vec<_> = m.faces().map(|(id, _)| id).collect();
    m.copy(&[], &faces, p(10.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    // The island copy is split in two; the opening around it stays open.
    assert_eq!(m.faces().count(), 5);
    assert!(m.faces().all(|(_, f)| f.normal == normal));
}

/// A 10 m square with a 1 m square drawn on it, which splits it in two.
fn square_drawn_on_a_face() -> Model {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 10.0);
    square(&mut m, 1.0, 1.0, 1.0);
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    m
}

#[test]
fn copying_bare_edges_onto_a_face_keeps_the_piece_they_cut_out() {
    let mut m = square_drawn_on_a_face();
    let small: Vec<_> = m
        .edges()
        .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x < 3.0))
        .filter(|(_, e)| e.v.iter().all(|v| m.position(*v).unwrap().x > 0.5))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(small.len(), 4);
    let before = m.clone();
    let c = m.copy(&small, &[], p(4.0, 0.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    // The copied outline splits the 10 m face again; the piece inside stays a face.
    assert_eq!(m.faces().count(), 3);
    let normal = face_normal_of_first(&before);
    assert!(m.faces().all(|(_, f)| f.normal == normal));
    let pieces = faces_at(&m, |q| q.x > 4.5 && q.x < 6.5);
    assert_eq!(pieces.len(), 1);
    assert!(m.face(pieces[0]).unwrap().holes.is_empty());
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(m, before);
    m.apply(&c);
    assert_eq!(m, after);
}

#[test]
fn a_face_with_an_opening_copied_onto_a_face_leaves_the_face_under_the_opening() {
    let mut m = Model::new();
    square(&mut m, 0.0, 0.0, 10.0);
    let source = square(&mut m, 20.0, 0.0, 4.0);
    let hole = square(&mut m, 21.5, 1.5, 1.0);
    m.erase(&[], &[hole]);
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 2);
    m.copy(&[], &[source], p(-18.0, 2.0, 0.0), 1).unwrap();
    m.validate().unwrap();
    // The copy splits the 10 m face; the piece under the copy's opening stays a face.
    let under = faces_at(&m, |q| {
        (3.4..4.6).contains(&q.x) && (3.4..4.6).contains(&q.y)
    });
    assert_eq!(under.len(), 1, "no hole is punched");
    assert_eq!(m.faces().count(), 4);
}
