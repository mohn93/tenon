use glam::DVec3;
use kernel::{EntityId, FaceId, KernelError, Model};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// Adds an axis-aligned rectangle at height z and returns its face.
fn rect(m: &mut Model, x0: f64, y0: f64, x1: f64, y1: f64, z: f64) -> FaceId {
    let c = m
        .add_polygon(&[p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)])
        .unwrap();
    m.validate().unwrap();
    let s = c.summary();
    let faces: Vec<FaceId> = s
        .created
        .iter()
        .filter_map(|id| match id {
            EntityId::Face(f) => Some(*f),
            _ => None,
        })
        .collect();
    *faces.last().expect("rectangle made a face")
}

/// The face whose plane is z = `z` and whose normal is parallel to Z.
fn face_at_height(m: &Model, z: f64) -> FaceId {
    m.faces()
        .find(|(_, f)| {
            f.normal.cross(DVec3::Z).length() < 1e-9
                && f.outer
                    .iter()
                    .all(|v| (m.position(*v).unwrap().z - z).abs() < 1e-9)
        })
        .map(|(id, _)| id)
        .expect("face at height")
}

fn counts(m: &Model) -> (usize, usize, usize) {
    (m.vertices().count(), m.edges().count(), m.faces().count())
}

/// Number of `solid` faces crossed by the ray from `from` in a fixed, generic direction.
fn crossings(m: &Model, solid: &[FaceId], from: DVec3) -> usize {
    let dir = DVec3::new(0.2113, 0.5773, 0.7887).normalize();
    let mut n = 0;
    for id in solid {
        for [a, b, c] in m.triangulate(*id).unwrap() {
            let (e1, e2) = (b - a, c - a);
            let h = dir.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-12 {
                continue;
            }
            let s = from - a;
            let u = s.dot(h) / det;
            let q = s.cross(e1);
            let v = dir.dot(q) / det;
            let t = e2.dot(q) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 1e-9 {
                n += 1;
            }
        }
    }
    n
}

/// Every face of the closed surface `solid` points out of it: just in front of the face is
/// outside (even crossings), just behind is inside (odd).
fn assert_outward(m: &Model, solid: &[FaceId]) {
    for id in solid {
        let f = m.face(*id).unwrap();
        let [a, b, c] = m.triangulate(*id).unwrap()[0];
        let centre = (a + b + c) / 3.0;
        let front = crossings(m, solid, centre + f.normal * 1e-4);
        let back = crossings(m, solid, centre - f.normal * 1e-4);
        assert!(
            front.is_multiple_of(2) && !back.is_multiple_of(2),
            "face {id:?} points inward"
        );
    }
}

fn all_faces(m: &Model) -> Vec<FaceId> {
    m.faces().map(|(id, _)| id).collect()
}

/// A 4x4x2 box pulled up from the ground. Returns the model and its top face.
fn box_4x4x2() -> (Model, FaceId) {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 4.0, 4.0, 0.0);
    m.push_pull(base, 2.0).unwrap();
    let top = face_at_height(&m, 2.0);
    (m, top)
}

#[test]
fn pulling_a_free_rectangle_makes_a_closed_box() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 3.0, 0.0);
    let c = m.push_pull(base, 1.5).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    assert_eq!(m.face(base).unwrap().normal, -DVec3::Z, "base faces down");
    assert_eq!(m.face(face_at_height(&m, 1.5)).unwrap().normal, DVec3::Z);
    assert_outward(&m, &all_faces(&m));
    let s = c.summary();
    assert_eq!(s.modified, vec![EntityId::Face(base)]);
    assert!(s.deleted.is_empty());
}

#[test]
fn pushing_a_free_rectangle_down_makes_a_box_below() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 3.0, 0.0);
    m.push_pull(base, -1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    assert_eq!(m.face(base).unwrap().normal, DVec3::Z, "base faces up");
    assert_eq!(m.face(face_at_height(&m, -1.0)).unwrap().normal, -DVec3::Z);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn pulling_a_box_top_stretches_the_sides() {
    let (mut m, top) = box_4x4x2();
    let c = m.push_pull(top, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(counts(&m), (8, 12, 6));
    assert!(m.face(top).is_some(), "the top keeps its id");
    assert_eq!(face_at_height(&m, 3.0), top);
    let s = c.summary();
    assert!(s.created.is_empty() && s.deleted.is_empty());
    assert_eq!(s.modified.len(), 4);
    assert!(
        s.modified
            .iter()
            .all(|id| matches!(id, EntityId::Vertex(_)))
    );
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn pushing_a_box_top_to_or_through_the_bottom_fails() {
    let (mut m, top) = box_4x4x2();
    let before = m.clone();
    assert_eq!(m.push_pull(top, -2.0), Err(KernelError::Collapse));
    assert_eq!(m.push_pull(top, -3.0), Err(KernelError::Collapse));
    assert_eq!(m, before);
    assert!(m.push_pull(top, -1.0).is_ok());
}

#[test]
fn rectangle_on_a_box_top_pulls_up_as_a_bump() {
    let (mut m, top) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, 1.0).unwrap();
    m.validate().unwrap();
    assert!(m.face(sub).is_none(), "the bump's base is not a face");
    assert_eq!(m.faces().count(), 11);
    assert_eq!(m.face(top).unwrap().holes.len(), 1);
    assert_eq!(m.face(face_at_height(&m, 3.0)).unwrap().normal, DVec3::Z);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn rectangle_on_a_box_top_pushed_down_cuts_a_recess() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    m.validate().unwrap();
    assert!(m.face(sub).is_none());
    assert_eq!(m.faces().count(), 11);
    assert_eq!(m.face(face_at_height(&m, 1.0)).unwrap().normal, DVec3::Z);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn face_with_a_hole_extrudes_a_tube() {
    let mut m = Model::new();
    let ring = rect(&mut m, 0.0, 0.0, 4.0, 4.0, 0.0);
    let inner = rect(&mut m, 1.0, 1.0, 3.0, 3.0, 0.0);
    assert_eq!(m.face(ring).unwrap().holes.len(), 1);
    m.push_pull(ring, 2.0).unwrap();
    m.validate().unwrap();
    assert_eq!(m.faces().count(), 11);
    let cap = face_at_height(&m, 2.0);
    assert_eq!(m.face(cap).unwrap().holes.len(), 1, "the hole stays open");
    assert!((m.area(cap).unwrap() - 12.0).abs() < 1e-9);
    let tube: Vec<FaceId> = all_faces(&m).into_iter().filter(|f| *f != inner).collect();
    assert_outward(&m, &tube);
}

#[test]
fn pulling_one_of_two_adjacent_rectangles_keeps_the_neighbor() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    let neighbor = m.face(b).unwrap().clone();
    m.push_pull(a, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(m.face(b), Some(&neighbor));
    assert_eq!(m.face(a).unwrap().normal, -DVec3::Z);
    assert_eq!(m.faces().count(), 7);
}

#[test]
fn walls_at_any_angle_extrude_into_boxes() {
    for (u, w) in [
        (DVec3::X, DVec3::Z),
        (DVec3::new(1.0, 1.0, 0.0).normalize(), DVec3::Z),
        (DVec3::X, DVec3::new(0.0, 1.0, 1.0).normalize()),
    ] {
        let mut m = Model::new();
        let o = p(0.3, 0.7, 0.0);
        let c = m
            .add_polygon(&[o, o + u * 2.0, o + u * 2.0 + w * 1.5, o + w * 1.5])
            .unwrap();
        let face = c
            .summary()
            .created
            .iter()
            .find_map(|id| match id {
                EntityId::Face(f) => Some(*f),
                _ => None,
            })
            .unwrap();
        m.push_pull(face, 0.8).unwrap();
        m.validate().unwrap();
        assert_eq!(counts(&m), (8, 12, 6), "wall along {u} and {w}");
        assert_outward(&m, &all_faces(&m));
    }
}

#[test]
fn push_pull_undoes_and_redoes_exactly() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let flat = m.clone();
    let extrude = m.push_pull(base, 1.0).unwrap();
    let boxed = m.clone();
    let top = face_at_height(&m, 1.0);
    let stretch = m.push_pull(top, 0.5).unwrap();
    let taller = m.clone();
    m.apply(&stretch.inverse());
    assert_eq!(m, boxed);
    m.apply(&extrude.inverse());
    assert_eq!(m, flat);
    m.apply(&extrude);
    m.apply(&stretch);
    assert_eq!(m, taller);
}

#[test]
fn bad_push_pull_input_changes_nothing() {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let before = m.clone();
    let missing = FaceId(999);
    assert_eq!(m.push_pull(missing, 1.0), Err(KernelError::FaceNotFound));
    assert_eq!(m.push_pull(base, 0.0), Err(KernelError::ZeroDistance));
    assert_eq!(m.push_pull(base, f64::NAN), Err(KernelError::NotFinite));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}
