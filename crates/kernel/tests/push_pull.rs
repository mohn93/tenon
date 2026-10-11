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

#[test]
fn cut_partly_covered_by_an_old_face_stays_closed() {
    // A step at y < 1, then a recess beside it cut deeper (C1) or shallower and deeper (C5)
    // than the step floor: only the part of the recess wall the old riser covered opens.
    for step in [-1.0, -0.5] {
        let (mut m, _) = box_4x4x2();
        rect(&mut m, 0.0, 0.0, 4.0, 1.0, 2.0);
        let strip = face_in(&m, 2.0, 0.0, 0.0, 4.0, 1.0);
        m.push_pull(strip, step).unwrap();
        m.validate().unwrap();
        rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
        let sub = face_in(&m, 2.0, 1.0, 1.0, 2.0, 2.0);
        push_pull_undoable(&mut m, sub, -1.5);
        m.validate().unwrap();
        assert_outward(&m, &all_faces(&m));
        assert_closed(&m, &[]);
    }
}

#[test]
fn cut_keeps_user_lines_it_splits() {
    let (mut m, _) = box_4x4x2();
    let (s, e) = (p(1.5, 0.0, 1.5), p(2.5, 0.0, 1.5));
    m.add_edge(s, e).unwrap();
    m.validate().unwrap();
    rect(&mut m, 1.0, 0.0, 2.0, 1.0, 2.0);
    let sub = face_in(&m, 2.0, 1.0, 0.0, 2.0, 1.0);
    push_pull_undoable(&mut m, sub, -1.0);
    m.validate().unwrap();
    assert_outward(&m, &all_faces(&m));
    for q in [s, e] {
        let v = m
            .vertices()
            .find(|(_, v)| v.pos.distance(q) < 1e-9)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("line end {q:?} is gone"));
        assert!(!m.edges_at(v).is_empty(), "line end {q:?} has no edge");
    }
    assert_no_isolated_vertices(&m);
}

/// A wall 4 m long, 0.3 m thick and 3 m high, pulled up from the ground.
fn wall() -> Model {
    let mut m = Model::new();
    let base = rect(&mut m, 0.0, 0.0, 4.0, 0.3, 0.0);
    m.push_pull(base, 3.0).unwrap();
    m.take_changes();
    m
}

/// Draws the rectangle x0..x1, z0..z1 on the wall's front face (y = 0) and returns its face.
fn window(m: &mut Model, x0: f64, z0: f64, x1: f64, z1: f64) -> FaceId {
    m.add_polygon(&[
        p(x0, 0.0, z0),
        p(x1, 0.0, z0),
        p(x1, 0.0, z1),
        p(x0, 0.0, z1),
    ])
    .unwrap();
    m.validate().unwrap();
    let e = 1e-9;
    let mut found = faces_where(m, |q| {
        q.y.abs() < e && q.x > x0 - e && q.x < x1 + e && q.z > z0 - e && q.z < z1 + e
    });
    assert_eq!(found.len(), 1, "one face in the window");
    found.pop().unwrap()
}

/// The faces a ray along +y through (x, z) meets.
fn faces_on_line(m: &Model, x: f64, z: f64) -> usize {
    m.faces()
        .filter(|(id, _)| {
            m.triangulate(*id).unwrap().iter().any(|[a, b, c]| {
                let n = (*b - *a).cross(*c - *a);
                if n.y.abs() < 1e-12 {
                    return false;
                }
                let q = p(x, 0.0, z);
                let t = n.dot(*a - q) / n.y;
                let hit = q + DVec3::Y * t;
                let inside = |u: DVec3, v: DVec3| (v - u).cross(hit - u).dot(n) >= 0.0;
                inside(*a, *b) && inside(*b, *c) && inside(*c, *a)
            })
        })
        .count()
}

#[test]
fn pushing_a_window_to_the_back_of_a_wall_opens_it() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    assert_eq!(faces_on_line(&m, 1.5, 1.5), 2, "front and back before");
    push_pull_undoable(&mut m, win, -0.3);
    m.validate().unwrap();
    assert_eq!(
        faces_on_line(&m, 1.5, 1.5),
        0,
        "nothing left in the opening"
    );
    assert_eq!(faces_on_line(&m, 0.5, 1.5), 2, "the wall beside it stays");
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    // The front and back keep a hole each; four reveals line the opening.
    assert_eq!(m.faces().count(), 6 + 4);
}

#[test]
fn pushing_a_window_at_the_top_of_a_wall_cuts_a_slot() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 2.0, 2.0, 3.0);
    push_pull_undoable(&mut m, win, -0.3);
    m.validate().unwrap();
    assert_eq!(faces_on_line(&m, 1.5, 2.5), 0, "the slot is open");
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    let tops = faces_where(&m, |q| (q.z - 3.0).abs() < 1e-9).len();
    assert_eq!(tops, 2, "the slot splits the top in two");
    for (id, e) in m.edges() {
        assert!(
            side_uses(&m, e.v[0], e.v[1], &[]) > 0,
            "edge {id:?} borders no face"
        );
    }
}

#[test]
fn pushing_a_window_past_the_back_of_a_wall_is_refused() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(win, -0.5), Err(KernelError::Obstructed));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn pushing_a_hole_through_a_box_from_the_top() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    push_pull_undoable(&mut m, sub, -2.0);
    m.validate().unwrap();
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    let caps = faces_where(&m, |q| {
        (q.x - 1.5).abs() < 0.5 + 1e-9 && (q.y - 1.5).abs() < 0.5 + 1e-9 && q.z.abs() < 1e-9
    });
    assert!(caps.is_empty(), "no face closes the bottom of the hole");
    assert_eq!(m.faces().count(), 6 + 4);
}

#[test]
fn a_window_drawn_on_both_sides_opens_both() {
    let mut m = wall();
    let back = [
        p(1.0, 0.3, 1.0),
        p(2.0, 0.3, 1.0),
        p(2.0, 0.3, 2.0),
        p(1.0, 0.3, 2.0),
    ];
    m.add_polygon(&back).unwrap();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, win, -0.3);
    m.validate().unwrap();
    assert_eq!(
        faces_on_line(&m, 1.5, 1.5),
        0,
        "nothing left in the opening"
    );
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn pushing_a_ring_through_a_box_leaves_a_column() {
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 1.0, 1.0, 3.0, 3.0, 2.0);
    rect(&mut m, 1.5, 1.5, 2.5, 2.5, 2.0);
    let ring = m
        .faces()
        .find(|(_, f)| {
            f.holes.len() == 1 && f.outer.len() == 4 && {
                let q = m.position(f.outer[0]).unwrap();
                (q.z - 2.0).abs() < 1e-9 && q.x > 0.5 && q.x < 3.5 && q.y > 0.5 && q.y < 3.5
            }
        })
        .map(|(id, _)| id)
        .expect("ring face");
    push_pull_undoable(&mut m, ring, -2.0);
    m.validate().unwrap();
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    // The column keeps its top and bottom.
    assert_eq!(
        faces_where(&m, |q| (q.x - 2.0).abs() < 0.5 + 1e-9
            && (q.y - 2.0).abs() < 0.5 + 1e-9
            && q.z.abs() < 1e-9)
        .len(),
        1
    );
}

/// Draws the square x0..x1, z0..z1 on the wall's back face (y = 0.3) and moves it by `depth`
/// outward (positive: a boss) or into the wall (negative: a pocket).
fn back_feature(m: &mut Model, x0: f64, z0: f64, x1: f64, z1: f64, depth: f64) {
    m.add_polygon(&[
        p(x0, 0.3, z0),
        p(x1, 0.3, z0),
        p(x1, 0.3, z1),
        p(x0, 0.3, z1),
    ])
    .unwrap();
    m.validate().unwrap();
    let e = 1e-9;
    let id = faces_where(m, |q| {
        (q.y - 0.3).abs() < e && q.x > x0 - e && q.x < x1 + e && q.z > z0 - e && q.z < z1 + e
    })
    .pop()
    .expect("feature face");
    let ny = m.faces().find(|(f, _)| *f == id).unwrap().1.normal.y;
    // The feature face points out of the wall (+y) or into it, depending on how it was split.
    m.push_pull(id, depth * ny.signum()).unwrap();
    m.validate().unwrap();
    m.take_changes();
}

/// Asserts that pushing `win` by -0.3 is refused and changes nothing.
fn assert_refused(m: &mut Model, win: FaceId) {
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(win, -0.3), Err(KernelError::Obstructed));
    assert_eq!(*m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn a_boss_on_the_back_inside_the_window_refuses_the_push() {
    let mut m = wall();
    back_feature(&mut m, 1.4, 1.4, 1.6, 1.6, 0.2);
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    assert_refused(&mut m, win);
}

#[test]
fn a_pocket_on_the_back_inside_the_window_refuses_the_push() {
    let mut m = wall();
    back_feature(&mut m, 1.4, 1.4, 1.6, 1.6, -0.1);
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    assert_refused(&mut m, win);
}

#[test]
fn a_notch_at_the_back_top_edge_inside_the_slot_refuses_the_push() {
    let mut m = wall();
    back_feature(&mut m, 1.1, 2.9, 1.3, 3.0, -0.1);
    let win = window(&mut m, 1.0, 2.5, 2.0, 3.0);
    assert_refused(&mut m, win);
}

#[test]
fn a_window_sticking_past_a_notched_back_refuses_the_push() {
    let mut m = wall();
    // The notch edge crosses the window's side: x 1.5..2.5 against the window x 1..2.
    back_feature(&mut m, 1.5, 2.9, 2.5, 3.0, -0.1);
    let win = window(&mut m, 1.0, 2.5, 2.0, 3.0);
    assert_refused(&mut m, win);
}

/// The model is exactly a plain box: 8 vertices, 12 edges and 6 faces pointing out.
fn assert_plain_box(m: &Model) {
    m.validate().unwrap();
    assert_eq!(counts(m), (8, 12, 6));
    assert!(
        m.faces()
            .all(|(_, f)| f.holes.is_empty() && f.outer.len() == 4)
    );
    assert_outward(m, &all_faces(m));
}

#[test]
fn pushing_a_recess_back_flush_closes_it() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    let floor = face_in(&m, 1.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, floor, 1.0);
    assert_plain_box(&m);
}

#[test]
fn pushing_a_bump_back_flush_removes_it() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, 1.0).unwrap();
    let top = face_in(&m, 3.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, top, -1.0);
    assert_plain_box(&m);
}

#[test]
fn pushing_notches_and_steps_back_flush_restores_the_box() {
    for (x1, y1) in [(1.0, 1.0), (4.0, 1.0), (2.0, 1.0)] {
        let (mut m, _) = box_4x4x2();
        rect(&mut m, 0.0, 0.0, x1, y1, 2.0);
        let sub = face_in(&m, 2.0, 0.0, 0.0, x1, y1);
        m.push_pull(sub, -1.0).unwrap();
        let floor = face_in(&m, 1.0, 0.0, 0.0, x1, y1);
        push_pull_undoable(&mut m, floor, 1.0);
        assert_plain_box(&m);
    }
}

#[test]
fn pushing_a_recess_past_flush_is_refused() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    let floor = face_in(&m, 1.0, 1.0, 1.0, 2.0, 2.0);
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(floor, 1.5), Err(KernelError::Collapse));
    assert_eq!(m, before);
}

#[test]
fn pulling_a_lower_box_level_with_its_neighbour_removes_the_step() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 2.0).unwrap();
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    m.push_pull(b, 1.0).unwrap();
    let top = face_in(&m, 1.0, 2.0, 0.0, 4.0, 2.0);
    push_pull_undoable(&mut m, top, 1.0);
    m.validate().unwrap();
    let step = faces_where(&m, |q| (q.x - 2.0).abs() < 1e-9 && q.z > 1.0 - 1e-9);
    assert!(step.is_empty(), "no step wall is left");
    let internal = faces_where(&m, |q| (q.x - 2.0).abs() < 1e-9);
    assert_eq!(internal.len(), 1, "one internal wall");
    let solid: Vec<FaceId> = all_faces(&m)
        .into_iter()
        .filter(|f| !internal.contains(f))
        .collect();
    assert_outward(&m, &solid);
    assert_closed(&m, &internal);
}

#[test]
fn a_window_recess_pushed_back_flush_closes() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    m.push_pull(win, -0.1).unwrap();
    let floor = faces_where(&m, |q| (q.y - 0.1).abs() < 1e-9)[0];
    push_pull_undoable(&mut m, floor, 0.1);
    assert_plain_box(&m);
}

#[test]
fn flush_push_keeps_a_line_that_ends_on_the_rim_without_overlaps() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    m.add_edge(DVec3::new(1.5, 1.0, 2.0), DVec3::new(1.5, 0.0, 2.0))
        .unwrap();
    let floor = face_in(&m, 1.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, floor, 1.0);
    m.validate().unwrap();
    // 12 box edges, the front edge split at (1.5,0) (+1) and the line (+1); the rim is gone.
    assert_eq!(counts(&m), (10, 14, 6));
    assert_outward(&m, &all_faces(&m));
}

/// The face in plane y = `y` whose vertices all lie in x0..x1, z0..z1.
fn face_at_depth(m: &Model, y: f64, x0: f64, z0: f64, x1: f64, z1: f64) -> FaceId {
    let e = 1e-9;
    let mut found = faces_where(m, |q| {
        (q.y - y).abs() < e && q.x > x0 - e && q.x < x1 + e && q.z > z0 - e && q.z < z1 + e
    });
    assert_eq!(found.len(), 1, "one face at that depth");
    found.pop().unwrap()
}

#[test]
fn pushing_a_recess_floor_on_to_the_back_of_a_wall_opens_it() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    m.push_pull(win, -0.1).unwrap();
    let floor = face_at_depth(&m, 0.1, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, floor, -0.2);
    m.validate().unwrap();
    assert_eq!(faces_on_line(&m, 1.5, 1.5), 0, "the opening is open");
    assert_eq!(faces_on_line(&m, 0.5, 1.5), 2, "the wall beside it stays");
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    assert_eq!(m.faces().count(), 6 + 4);
    assert_no_isolated_vertices(&m);
}

#[test]
fn pushing_a_slot_floor_on_to_the_back_of_a_wall_opens_it() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 2.0, 2.0, 3.0);
    m.push_pull(win, -0.1).unwrap();
    let floor = face_at_depth(&m, 0.1, 1.0, 2.0, 2.0, 3.0);
    push_pull_undoable(&mut m, floor, -0.2);
    m.validate().unwrap();
    assert_eq!(faces_on_line(&m, 1.5, 2.5), 0, "the slot is open");
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    let tops = faces_where(&m, |q| (q.z - 3.0).abs() < 1e-9).len();
    assert_eq!(tops, 2, "the slot splits the top in two");
}

#[test]
fn pushing_a_hole_floor_on_through_a_box() {
    let (mut m, _) = box_4x4x2();
    let sub = rect(&mut m, 1.0, 1.0, 2.0, 2.0, 2.0);
    m.push_pull(sub, -1.0).unwrap();
    let floor = face_in(&m, 1.0, 1.0, 1.0, 2.0, 2.0);
    push_pull_undoable(&mut m, floor, -1.0);
    m.validate().unwrap();
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
    let caps = faces_where(&m, |q| {
        (q.x - 1.5).abs() < 0.5 + 1e-9 && (q.y - 1.5).abs() < 0.5 + 1e-9 && q.z.abs() < 1e-9
    });
    assert!(caps.is_empty(), "no face closes the bottom of the hole");
    assert_eq!(m.faces().count(), 6 + 4);
    assert_no_isolated_vertices(&m);
}

#[test]
fn pushing_a_recess_floor_past_the_back_is_refused() {
    let mut m = wall();
    let win = window(&mut m, 1.0, 1.0, 2.0, 2.0);
    m.push_pull(win, -0.1).unwrap();
    let floor = face_at_depth(&m, 0.1, 1.0, 1.0, 2.0, 2.0);
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(floor, -0.4), Err(KernelError::Obstructed));
    assert_eq!(m, before);
    assert!(m.take_changes().is_empty());
}

/// Asserts that pushing `id` by `d` fails with `err` and changes nothing.
fn assert_push_fails(m: &mut Model, id: FaceId, d: f64, err: KernelError) {
    m.take_changes();
    let before = m.clone();
    assert_eq!(m.push_pull(id, d), Err(err));
    assert_eq!(*m, before);
    assert!(m.take_changes().is_empty());
}

#[test]
fn full_width_strip_pushed_to_the_bottom_is_refused() {
    // The cut would match the whole front face, which would be left floating.
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 0.0, 0.0, 4.0, 1.0, 2.0);
    let strip = face_in(&m, 2.0, 0.0, 0.0, 4.0, 1.0);
    assert_push_fails(&mut m, strip, -2.0, KernelError::Obstructed);
}

#[test]
fn lower_box_top_pushed_to_the_ground_beside_a_taller_box_collapses() {
    let mut m = Model::new();
    let a = rect(&mut m, 0.0, 0.0, 2.0, 2.0, 0.0);
    m.push_pull(a, 1.0).unwrap();
    let b = rect(&mut m, 2.0, 0.0, 4.0, 2.0, 0.0);
    m.push_pull(b, 1.5).unwrap();
    let top = face_in(&m, 1.0, 0.0, 0.0, 2.0, 2.0);
    assert_push_fails(&mut m, top, -1.0, KernelError::Collapse);
}

#[test]
fn full_width_tread_pushed_to_the_bottom_collapses() {
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 0.0, 0.0, 4.0, 1.0, 2.0);
    let strip = face_in(&m, 2.0, 0.0, 0.0, 4.0, 1.0);
    m.push_pull(strip, -1.0).unwrap();
    let tread = face_in(&m, 1.0, 0.0, 0.0, 4.0, 1.0);
    assert_push_fails(&mut m, tread, -1.0, KernelError::Collapse);
}

#[test]
fn corner_tread_pushed_to_the_bottom_opens_a_closed_notch() {
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 0.0, 0.0, 1.0, 1.0, 2.0);
    let corner = face_in(&m, 2.0, 0.0, 0.0, 1.0, 1.0);
    m.push_pull(corner, -1.0).unwrap();
    let tread = face_in(&m, 1.0, 0.0, 0.0, 1.0, 1.0);
    push_pull_undoable(&mut m, tread, -1.0);
    m.validate().unwrap();
    assert_closed(&m, &[]);
    assert_outward(&m, &all_faces(&m));
}

#[test]
fn ring_pushed_through_around_a_recess_is_refused() {
    // The hole side's sweep would match the old recess walls, which would be left floating.
    let (mut m, _) = box_4x4x2();
    rect(&mut m, 1.0, 1.0, 3.0, 3.0, 2.0);
    rect(&mut m, 1.5, 1.5, 2.5, 2.5, 2.0);
    let inner = face_in(&m, 2.0, 1.5, 1.5, 2.5, 2.5);
    m.push_pull(inner, -1.0).unwrap();
    let ring = face_in(&m, 2.0, 1.0, 1.0, 3.0, 3.0);
    assert_push_fails(&mut m, ring, -2.0, KernelError::Obstructed);
}
