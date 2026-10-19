use std::collections::BTreeSet;

use glam::{DAffine3, DQuat, DVec3};
use kernel::geom::vector_area;
use kernel::{EntityId, FaceId, KernelError, Model, VertexId};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 1 m box on the ground at the origin, built with push/pull.
fn unit_box() -> Model {
    let mut m = Model::new();
    let c = m
        .add_polygon(&[
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
        ])
        .unwrap();
    let base = c
        .summary()
        .created
        .iter()
        .find_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .unwrap();
    m.push_pull(base, 1.0).unwrap();
    m
}

fn all_vertices(m: &Model) -> BTreeSet<VertexId> {
    m.vertices().map(|(id, _)| id).collect()
}

fn bounds(m: &Model) -> (DVec3, DVec3) {
    m.vertices().fold(
        (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)),
        |(lo, hi), (_, v)| (lo.min(v.pos), hi.max(v.pos)),
    )
}

fn about(origin: DVec3, factors: DVec3) -> DAffine3 {
    DAffine3::from_translation(origin)
        * DAffine3::from_scale(factors)
        * DAffine3::from_translation(-origin)
}

fn face_with_normal(m: &Model, n: DVec3) -> FaceId {
    m.faces()
        .find(|(_, f)| f.normal.distance(n) < 1e-9)
        .map(|(id, _)| id)
        .unwrap()
}

#[test]
fn scaling_a_box_doubles_it() {
    let mut m = unit_box();
    let before: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    let all = all_vertices(&m);
    m.transform_vertices(&all, about(DVec3::ZERO, DVec3::splat(2.0)))
        .unwrap();
    m.validate().unwrap();
    assert_eq!(bounds(&m), (DVec3::ZERO, DVec3::splat(2.0)));
    let after: Vec<DVec3> = m.faces().map(|(_, f)| f.normal).collect();
    assert_eq!(before, after);
}

#[test]
fn stretching_keeps_faces_flat_and_normals_right() {
    let mut m = unit_box();
    let all = all_vertices(&m);
    m.transform_vertices(&all, about(DVec3::ZERO, p(3.0, 1.0, 1.0)))
        .unwrap();
    m.validate().unwrap();
    assert_eq!(bounds(&m), (DVec3::ZERO, p(3.0, 1.0, 1.0)));
    for n in [
        DVec3::X,
        -DVec3::X,
        DVec3::Y,
        -DVec3::Y,
        DVec3::Z,
        -DVec3::Z,
    ] {
        face_with_normal(&m, n);
    }
    for (_, f) in m.faces() {
        assert!((f.normal.length() - 1.0).abs() < 1e-12);
    }
}

#[test]
fn mirroring_keeps_faces_outward() {
    let mut m = unit_box();
    let center = p(0.5, 0.5, 0.5);
    let all = all_vertices(&m);
    m.transform_vertices(&all, about(center, p(-1.0, 1.0, 1.0)))
        .unwrap();
    m.validate().unwrap();
    assert_eq!(bounds(&m), (DVec3::ZERO, DVec3::ONE));
    for (_, f) in m.faces() {
        let outer: Vec<DVec3> = f.outer.iter().map(|v| m.vertex(*v).unwrap().pos).collect();
        let mid = outer.iter().copied().sum::<DVec3>() / outer.len() as f64;
        assert!(f.normal.dot(mid - center) > 0.0, "face points out");
        let area = vector_area(&outer);
        assert!(
            area.normalize().distance(f.normal) < 1e-9,
            "outer loop winds counter-clockwise around the normal"
        );
    }
    let top = face_with_normal(&m, DVec3::Z);
    m.push_pull(top, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(bounds(&m), (DVec3::ZERO, p(1.0, 1.0, 2.0)));
}

#[test]
fn shrinking_an_edge_below_tolerance_is_refused() {
    let mut m = unit_box();
    let all = all_vertices(&m);
    let before = m.clone();
    assert!(matches!(
        m.transform_vertices(&all, about(DVec3::ZERO, p(1e-9, 1.0, 1.0))),
        Err(KernelError::Squash)
    ));
    assert_eq!(bounds(&m), bounds(&before));
    assert!(matches!(
        m.transform_vertices(&all, about(DVec3::ZERO, p(0.0, 1.0, 1.0))),
        Err(KernelError::Squash)
    ));
}

#[test]
fn rotations_are_unchanged() {
    let mut m = unit_box();
    let t = DAffine3::from_rotation_translation(
        DQuat::from_axis_angle(p(1.0, 2.0, 3.0).normalize(), 0.7),
        p(0.3, -0.2, 1.1),
    );
    let positions: Vec<DVec3> = m
        .vertices()
        .map(|(_, v)| t.transform_point3(v.pos))
        .collect();
    let normals: Vec<DVec3> = m
        .faces()
        .map(|(_, f)| t.transform_vector3(f.normal).normalize())
        .collect();
    let outers: Vec<Vec<VertexId>> = m.faces().map(|(_, f)| f.outer.clone()).collect();
    let all = all_vertices(&m);
    m.transform_vertices(&all, t).unwrap();
    m.validate().unwrap();
    assert_eq!(
        m.vertices().map(|(_, v)| v.pos).collect::<Vec<_>>(),
        positions
    );
    assert_eq!(
        m.faces().map(|(_, f)| f.normal).collect::<Vec<_>>(),
        normals
    );
    assert_eq!(
        m.faces().map(|(_, f)| f.outer.clone()).collect::<Vec<_>>(),
        outers
    );
}

/// A 10 x 10 square on the ground with a hole from (x0, 4) to (x1, 6); returns the model
/// and the face around the hole.
fn square_with_hole(x0: f64, x1: f64) -> (Model, FaceId) {
    let mut m = Model::new();
    let ring =
        |a: f64, b: f64, c: f64, d: f64| [p(a, c, 0.0), p(b, c, 0.0), p(b, d, 0.0), p(a, d, 0.0)];
    m.add_polygon(&ring(0.0, 10.0, 0.0, 10.0)).unwrap();
    m.add_polygon(&ring(x0, x1, 4.0, 6.0)).unwrap();
    let outer = m
        .faces()
        .find(|(_, f)| !f.holes.is_empty())
        .map(|(id, _)| id)
        .unwrap();
    (m, outer)
}

#[test]
fn unconnected_vertices_squeezed_together_are_refused() {
    let mut m = Model::new();
    m.add_edge(p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0)).unwrap();
    m.add_edge(p(0.005, 0.0, 0.0), p(0.005, 1.0, 0.0)).unwrap();
    let all = all_vertices(&m);
    let before = m.clone();
    assert!(matches!(
        m.transform_vertices(&all, about(DVec3::ZERO, p(1e-4, 1.0, 1.0))),
        Err(KernelError::Squash)
    ));
    assert_eq!(m, before);
    m.validate().unwrap();
}

#[test]
fn a_hole_squeezed_onto_its_outer_ring_is_refused() {
    // The hole's left side is 5 mm from the outer ring's; scaled by 1e-4 along x it would be
    // 0.0005 mm away, while every edge keeps a length and no vertices meet.
    let (mut m, _) = square_with_hole(0.005, 5.0);
    let all = all_vertices(&m);
    let before = m.clone();
    assert!(matches!(
        m.transform_vertices(&all, about(DVec3::ZERO, p(1e-4, 1.0, 1.0))),
        Err(KernelError::Squash)
    ));
    assert_eq!(m, before);
}

#[test]
fn a_mirrored_face_with_a_hole_keeps_its_windings() {
    let (mut m, face) = square_with_hole(4.0, 6.0);
    let all = all_vertices(&m);
    m.transform_vertices(&all, about(p(5.0, 5.0, 0.0), p(-1.0, 1.0, 1.0)))
        .unwrap();
    m.validate().unwrap();
    let f = m.face(face).unwrap();
    let pts =
        |r: &Vec<VertexId>| -> Vec<DVec3> { r.iter().map(|v| m.vertex(*v).unwrap().pos).collect() };
    assert!(
        vector_area(&pts(&f.outer)).dot(f.normal) > 0.0,
        "outer counter-clockwise"
    );
    assert_eq!(f.holes.len(), 1);
    assert!(
        vector_area(&pts(&f.holes[0])).dot(f.normal) < 0.0,
        "hole clockwise"
    );
    // Every vertex was a mirror image of another on x = 5, so the outline is where it was.
    assert_eq!(bounds(&m), (DVec3::ZERO, p(10.0, 10.0, 0.0)));
}

#[test]
fn a_mirror_turns_over_a_face_on_the_mirror_plane() {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(1.0, 0.0, 0.0),
        p(1.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let (id, before) = m.faces().map(|(id, f)| (id, f.normal)).next().unwrap();
    let all = all_vertices(&m);
    m.transform_vertices(&all, about(DVec3::ZERO, p(1.0, 1.0, -1.0)))
        .unwrap();
    m.validate().unwrap();
    assert_eq!(m.face(id).unwrap().normal, -before);
}
