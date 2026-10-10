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
    assert!(m.take_changes().is_empty());
    assert!(m.push_pull(top, -1.0).is_ok());
}

#[test]
fn pushing_a_box_top_past_a_window_fails() {
    let (mut m, top) = box_4x4x2();
    m.add_polygon(&[
        p(1.0, 0.0, 1.0),
        p(2.0, 0.0, 1.0),
        p(2.0, 0.0, 1.8),
        p(1.0, 0.0, 1.8),
    ])
    .unwrap();
    m.validate().unwrap();
    let before = m.clone();
    assert_eq!(m.push_pull(top, -1.5), Err(KernelError::Collapse));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
    assert!(m.push_pull(top, -0.1).is_ok());
    m.validate().unwrap();
}

#[test]
fn pulling_the_second_of_two_adjacent_rectangles_keeps_its_floor() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    m.push_pull(a, 1.0).unwrap();
    m.push_pull(b, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(m.face(b).unwrap().normal, -DVec3::Z, "b keeps its floor");
    assert_eq!(m.face(a).unwrap().normal, -DVec3::Z);
    // The wall between the two boxes lies in plane x = 2 and is internal: skip it.
    let on_shared_wall = |f: &FaceId| {
        m.face(*f)
            .unwrap()
            .outer
            .iter()
            .all(|v| (m.position(*v).unwrap().x - 2.0).abs() < 1e-9)
    };
    let solid: Vec<FaceId> = all_faces(&m)
        .into_iter()
        .filter(|f| !on_shared_wall(f))
        .collect();
    assert_outward(&m, &solid);
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

/// Faces whose vertices all satisfy `on`.
fn faces_where(m: &Model, on: impl Fn(DVec3) -> bool) -> Vec<FaceId> {
    m.faces()
        .filter(|(_, f)| f.outer.iter().all(|v| on(m.position(*v).unwrap())))
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn rectangle_drawn_beside_a_box_pulls_up_into_a_closed_box() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 1.0).unwrap();
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    assert_eq!(
        m.face(b).unwrap().normal,
        DVec3::Z,
        "a new rectangle faces up"
    );
    m.push_pull(b, 1.0).unwrap();
    m.validate().unwrap();
    assert_eq!(
        m.face(b).unwrap().normal,
        -DVec3::Z,
        "b is kept as the floor"
    );
    let shared = faces_where(&m, |q| (q.x - 2.0).abs() < 1e-9);
    let solid: Vec<FaceId> = all_faces(&m)
        .into_iter()
        .filter(|f| !shared.contains(f))
        .collect();
    assert_outward(&m, &solid);
}

#[test]
fn splitting_a_flipped_base_keeps_it_facing_out() {
    let mut m = Model::new();
    let c = rect(&mut m, 10.0, 0.0, 12.0, 2.0, 0.0);
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 1.0).unwrap();
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 2.0, 0.0)).unwrap();
    m.validate().unwrap();
    let base: Vec<FaceId> = faces_where(&m, |q| q.z.abs() < 1e-9 && q.x < 5.0);
    assert_eq!(base.len(), 2);
    for f in &base {
        assert_eq!(m.face(*f).unwrap().normal, -DVec3::Z, "base piece {f:?}");
    }
    let solid: Vec<FaceId> = all_faces(&m).into_iter().filter(|f| *f != c).collect();
    assert_outward(&m, &solid);
}

#[test]
fn notch_at_a_box_edge_leaves_an_open_step() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 0.0, 2.0, 1.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    m.validate().unwrap();
    assert_outward(&m, &all_faces(&m));
    let front: f64 = faces_where(&m, |q| q.y.abs() < 1e-9)
        .iter()
        .map(|f| m.area(*f).unwrap())
        .sum();
    assert!((front - 7.0).abs() < 1e-9, "front area {front}");
    // The edge across the open notch went with its side face.
    let bordered = |a, b| {
        m.faces().any(|(_, f)| {
            std::iter::once(&f.outer).chain(&f.holes).any(|l| {
                (0..l.len()).any(|i| {
                    let s = (l[i], l[(i + 1) % l.len()]);
                    s == (a, b) || s == (b, a)
                })
            })
        })
    };
    assert!(m.edges().all(|(_, e)| bordered(e.v[0], e.v[1])));
}

#[test]
fn pushing_through_a_solid_is_refused() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(sub, -3.0), Err(KernelError::Obstructed));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());

    m.push_pull(sub, -1.0).unwrap();
    let bottom = face_at_height(&m, 1.0);
    let before = m.clone();
    assert_eq!(m.push_pull(bottom, -2.0), Err(KernelError::Obstructed));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn collapse_when_a_moved_vertex_lands_on_another() {
    let (mut m, top) = box_4x4x2();
    // A loose edge whose end sits 1 m above a top corner; nothing joins it to the box.
    m.add_edge(p(0.0, 0.0, 3.0), p(-1.0, -1.0, 3.0)).unwrap();
    m.validate().unwrap();
    let before = m.clone();
    assert_eq!(m.push_pull(top, 1.0), Err(KernelError::Collapse));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
    assert!(m.push_pull(top, 0.5).is_ok());
}

#[test]
fn in_solid_face_with_a_hole_pulls_into_a_tube() {
    let (mut m, _) = box_4x4x2();
    let ring = rect(&mut m, 1.0, 1.0, 3.0, 3.0, 2.0);
    let inner = rect(&mut m, 1.5, 1.5, 2.5, 2.5, 2.0);
    assert_eq!(m.face(ring).unwrap().holes.len(), 1);
    m.push_pull(ring, 1.0).unwrap();
    m.validate().unwrap();
    assert_outward(&m, &all_faces(&m));
    let f = m.face(inner).expect("the inner face stays");
    assert!(
        f.outer
            .iter()
            .all(|v| (m.position(*v).unwrap().z - 2.0).abs() < 1e-9)
    );
}

#[test]
fn loop_drawn_beside_a_flipped_base_faces_up() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 1.0).unwrap();
    let b = rect(&mut m, 5.0, 5.0, 6.0, 6.0, 0.0);
    assert_eq!(m.face(b).unwrap().normal, DVec3::Z);
}

/// The face in plane z = `z` whose vertices all lie in the box [x0, x1] x [y0, y1].
fn face_in(m: &Model, z: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> FaceId {
    let e = 1e-9;
    let mut found = faces_where(m, |q| {
        (q.z - z).abs() < e && q.x > x0 - e && q.x < x1 + e && q.y > y0 - e && q.y < y1 + e
    });
    assert_eq!(found.len(), 1, "one face in the region");
    found.pop().unwrap()
}

/// Number of face loops that hold `a`-`b` as a side, among faces not in `skip`.
fn side_uses(m: &Model, a: kernel::VertexId, b: kernel::VertexId, skip: &[FaceId]) -> usize {
    m.faces()
        .filter(|(id, _)| !skip.contains(id))
        .map(|(_, f)| {
            std::iter::once(&f.outer)
                .chain(&f.holes)
                .map(|l| {
                    (0..l.len())
                        .filter(|&i| {
                            let s = (l[i], l[(i + 1) % l.len()]);
                            s == (a, b) || s == (b, a)
                        })
                        .count()
                })
                .sum::<usize>()
        })
        .sum()
}

/// The model is a closed surface: every edge borders exactly two faces outside `internal`
/// (edges only internal faces border are skipped), and every vertex has an edge.
fn assert_closed(m: &Model, internal: &[FaceId]) {
    for (id, e) in m.edges() {
        let [a, b] = e.v;
        let uses = side_uses(m, a, b, internal);
        let internal_only = uses == 0 && side_uses(m, a, b, &[]) > 0;
        assert!(
            uses == 2 || internal_only,
            "edge {id:?} {:?}-{:?} borders {uses} faces",
            m.position(a).unwrap(),
            m.position(b).unwrap()
        );
    }
    assert_no_isolated_vertices(m);
}

fn assert_no_isolated_vertices(m: &Model) {
    for (v, vert) in m.vertices() {
        assert!(
            !m.edges_at(v).is_empty(),
            "vertex {v:?} at {:?} has no edge",
            vert.pos
        );
    }
}

/// Push/pulls `id` by `d`, checks the inverse restores the model exactly, then redoes it.
fn push_pull_undoable(m: &mut Model, id: FaceId, d: f64) {
    let before = m.clone();
    let c = m.push_pull(id, d).unwrap();
    let after = m.clone();
    m.apply(&c.inverse());
    assert_eq!(*m, before, "undo is exact");
    m.apply(&c);
    assert_eq!(*m, after, "redo is exact");
}

#[test]
fn step_wall_between_boxes_of_different_heights_faces_out() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 1.0).unwrap();
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    assert_eq!(m.face(b).unwrap().normal, DVec3::Z);
    push_pull_undoable(&mut m, b, 1.5);
    m.validate().unwrap();
    let on_wall = |q: DVec3| (q.x - 2.0).abs() < 1e-9;
    let internal = faces_where(&m, |q| on_wall(q) && q.z < 1.0 + 1e-9);
    assert_eq!(internal.len(), 1, "one internal wall piece");
    let step = faces_where(&m, |q| on_wall(q) && q.z > 1.0 - 1e-9);
    assert_eq!(step.len(), 1, "one step wall");
    assert_eq!(
        m.face(step[0]).unwrap().normal,
        -DVec3::X,
        "the step wall faces -X"
    );
    let solid: Vec<FaceId> = all_faces(&m)
        .into_iter()
        .filter(|f| !internal.contains(f))
        .collect();
    assert_outward(&m, &solid);
}

#[test]
fn cut_beside_a_rising_wall_keeps_the_recess_closed() {
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 0.0, 0.0, 4.0, 1.0, 2.0);
    let strip = face_in(&m, 2.0, 0.0, 0.0, 4.0, 1.0);
    m.push_pull(strip, 2.0).unwrap();
    m.validate().unwrap();
    rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    let sub = face_in(&m, 2.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, sub, -1.0);
    m.validate().unwrap();
    assert_outward(&m, &all_faces(&m));
    assert_closed(&m, &[]);
}

#[test]
fn corner_notch_and_full_width_step_leave_no_stray_edges() {
    for (x1, y1) in [(1.0, 1.0), (4.0, 1.0)] {
        let (mut m, _) = box_4x4x2();
        rect(&mut m, 0.0, 0.0, x1, y1, 2.0);
        let sub = face_in(&m, 2.0, 0.0, 0.0, x1, y1);
        push_pull_undoable(&mut m, sub, -1.0);
        m.validate().unwrap();
        assert_outward(&m, &all_faces(&m));
        for (id, e) in m.edges() {
            assert!(
                side_uses(&m, e.v[0], e.v[1], &[]) > 0,
                "notch to ({x1}, {y1}): edge {id:?} {:?}-{:?} borders no face",
                m.position(e.v[0]).unwrap(),
                m.position(e.v[1]).unwrap()
            );
        }
        assert_no_isolated_vertices(&m);
    }
}
